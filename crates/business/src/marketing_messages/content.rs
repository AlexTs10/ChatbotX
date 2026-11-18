//! `mmContentSchema` (`partials/facebook-marketing-message-content.ts`) and
//! `buildMarketingMessagePayload` / `summarizeMarketingMessage`
//! (`facebook-marketing-message-delivery/content-payload.ts`).

use chatbotx_core::flow_config::deps::zod_url_with_variables;
use chatbotx_core::flow_config::util::{ButtonPayload, encode_button_payload};
use chatbotx_core::flow_config::z::{self, Schema};
use serde_json::{Map, Value, json};

const MAX_QUICK_REPLIES: usize = 13;
const TEXT_MAX: usize = 640;
const ELEMENT_TITLE_MAX: usize = 80;
const ELEMENT_SUBTITLE_MAX: usize = 250;
const MAX_BUTTONS: usize = 3;
const MAX_ELEMENTS: usize = 10;
const QUICK_REPLY_TITLE_MAX: usize = 20;
/// `BUTTON_LABEL_MAX`.
const BUTTON_LABEL_MAX: usize = 20;

fn digits() -> Schema {
    z::string().regex(r"^\d+$", "")
}

fn external_flow() -> Schema {
    z::object(vec![
        ("actionType", z::literal(json!("startExternalFlow"))),
        ("flowId", digits()),
    ])
}

fn external_node() -> Schema {
    z::object(vec![
        ("actionType", z::literal(json!("startExternalNode"))),
        ("flowId", digits()),
        ("nodeId", digits()),
    ])
}

fn postback_action() -> Schema {
    z::discriminated_union("actionType", vec![external_flow(), external_node()])
}

fn button() -> Schema {
    z::object(vec![
        ("id", digits()),
        ("title", z::string().trim().min(1).max(BUTTON_LABEL_MAX)),
    ])
    .and(z::discriminated_union(
        "actionType",
        vec![
            z::object(vec![
                ("actionType", z::literal(json!("openWebsite"))),
                ("url", zod_url_with_variables(None)),
                (
                    "browserSize",
                    z::literals(vec![json!(40), json!(70), json!(100)]),
                ),
            ]),
            external_flow(),
            external_node(),
        ],
    ))
}

fn quick_reply() -> Schema {
    z::discriminated_union(
        "contentType",
        vec![
            z::object(vec![
                ("contentType", z::literal(json!("text"))),
                ("id", digits()),
                (
                    "title",
                    z::string().trim().min(1).max(QUICK_REPLY_TITLE_MAX),
                ),
                ("action", postback_action()),
            ]),
            z::object(vec![
                ("contentType", z::literal(json!("user_phone_number"))),
                ("id", digits()),
                ("action", postback_action().optional()),
            ]),
            z::object(vec![
                ("contentType", z::literal(json!("user_email"))),
                ("id", digits()),
                ("action", postback_action().optional()),
            ]),
        ],
    )
}

fn media_ref(url: Schema) -> Schema {
    z::object(vec![
        ("id", digits()),
        ("mode", z::enum_(&["url", "file"])),
        ("url", url),
        ("attachmentId", z::string().optional()),
    ])
}

fn quick_replies() -> Schema {
    z::array(quick_reply()).max(MAX_QUICK_REPLIES)
}

fn text() -> Schema {
    z::string().trim().min(1).max(TEXT_MAX)
}

/// `mmContentSchema`.
pub fn content_schema() -> Schema {
    z::discriminated_union(
        "templateType",
        vec![
            z::object(vec![
                ("templateType", z::literal(json!("text"))),
                ("text", text()),
                ("quickReplies", quick_replies()),
            ]),
            z::object(vec![
                ("templateType", z::literal(json!("button"))),
                ("text", text()),
                ("buttons", z::array(button()).min(1).max(MAX_BUTTONS)),
                ("quickReplies", quick_replies()),
            ]),
            z::object(vec![
                ("templateType", z::literal(json!("media"))),
                ("mediaType", z::enum_(&["image", "video"])),
                ("media", media_ref(zod_url_with_variables(None))),
                ("buttons", z::array(button()).max(MAX_BUTTONS)),
                ("quickReplies", quick_replies()),
            ]),
            z::object(vec![
                ("templateType", z::literal(json!("generic"))),
                ("greeting", z::string().trim().max(TEXT_MAX).optional()),
                (
                    "elements",
                    z::array(z::object(vec![
                        ("id", digits()),
                        ("title", z::string().trim().min(1).max(ELEMENT_TITLE_MAX)),
                        (
                            "subtitle",
                            z::string().trim().max(ELEMENT_SUBTITLE_MAX).optional(),
                        ),
                        (
                            "image",
                            media_ref(z::union(vec![
                                zod_url_with_variables(None),
                                z::literal(json!("")),
                            ]))
                            .optional(),
                        ),
                        ("buttons", z::array(button()).max(MAX_BUTTONS)),
                    ]))
                    .min(1)
                    .max(MAX_ELEMENTS),
                ),
                ("quickReplies", quick_replies()),
            ]),
        ],
    )
}

/// The parsed content (`mmContentSchema.safeParse(...).data`), `None` when
/// the stored value is not valid.
pub fn parse(content: &Value) -> Option<Value> {
    let parsed = content_schema().safe_parse(content);
    if parsed.success { parsed.data } else { None }
}

fn str_of<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or_default()
}

fn encode_postback(action: &Value) -> String {
    encode_button_payload(&ButtonPayload {
        flow_id: str_of(action, "flowId").to_owned(),
        node_id: (str_of(action, "actionType") == "startExternalNode")
            .then(|| str_of(action, "nodeId").to_owned()),
        ..ButtonPayload::default()
    })
}

fn convert_button(button: &Value) -> Value {
    if str_of(button, "actionType") == "openWebsite" {
        let ratio = match button["browserSize"].as_i64() {
            Some(40) => "compact",
            Some(70) => "tall",
            _ => "full",
        };
        return json!({
            "type": "web_url",
            "title": button["title"],
            "url": button["url"],
            "webview_height_ratio": ratio,
        });
    }
    json!({ "type": "postback", "title": button["title"], "payload": encode_postback(button) })
}

fn convert_quick_reply(qr: &Value) -> Value {
    let mut out = Map::new();
    out.insert("content_type".into(), qr["contentType"].clone());
    if str_of(qr, "contentType") == "text" {
        out.insert("title".into(), qr["title"].clone());
    }
    if qr["action"].is_object() {
        out.insert("payload".into(), json!(encode_postback(&qr["action"])));
    }
    Value::Object(out)
}

fn buttons_of(value: &Value) -> Vec<Value> {
    value["buttons"]
        .as_array()
        .map(|b| b.iter().map(convert_button).collect())
        .unwrap_or_default()
}

fn template(payload: Value) -> Value {
    json!({ "attachment": { "type": "template", "payload": payload } })
}

/// `buildMarketingMessagePayload` (of parsed content).
pub fn build_payload(content: &Value) -> Value {
    let mut message = match str_of(content, "templateType") {
        "text" => json!({ "text": content["text"] }),
        "button" => template(json!({
            "template_type": "button",
            "text": content["text"],
            "buttons": buttons_of(content),
        })),
        "media" => {
            let mut element = json!({ "media_type": content["mediaType"] });
            match content["media"]["attachmentId"]
                .as_str()
                .filter(|s| !s.is_empty())
            {
                Some(id) => element["attachment_id"] = json!(id),
                None => element["url"] = content["media"]["url"].clone(),
            }
            let buttons = buttons_of(content);
            if !buttons.is_empty() {
                element["buttons"] = json!(buttons);
            }
            template(json!({ "template_type": "media", "elements": [element] }))
        }
        _ => {
            let elements: Vec<Value> = content["elements"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|e| {
                    let mut out = json!({ "title": e["title"] });
                    if let Some(s) = e["subtitle"].as_str().filter(|s| !s.is_empty()) {
                        out["subtitle"] = json!(s);
                    }
                    if let Some(u) = e["image"]["url"].as_str().filter(|s| !s.is_empty()) {
                        out["image_url"] = json!(u);
                    }
                    let buttons = buttons_of(e);
                    if !buttons.is_empty() {
                        out["buttons"] = json!(buttons);
                    }
                    out
                })
                .collect();
            let mut payload = json!({ "template_type": "generic" });
            if let Some(g) = content["greeting"].as_str().filter(|s| !s.is_empty()) {
                payload["text"] = json!(g);
            }
            payload["elements"] = json!(elements);
            template(payload)
        }
    };
    if let Some(qrs) = content["quickReplies"].as_array().filter(|q| !q.is_empty()) {
        message["quick_replies"] = json!(qrs.iter().map(convert_quick_reply).collect::<Vec<_>>());
    }
    message
}

/// `summarizeMarketingMessage`.
pub fn summarize(content: &Value) -> String {
    match str_of(content, "templateType") {
        "text" | "button" => str_of(content, "text").to_owned(),
        "generic" => Some(str_of(content, "greeting"))
            .filter(|g| !g.is_empty())
            .or_else(|| content["elements"][0]["title"].as_str())
            .unwrap_or_default()
            .to_owned(),
        _ => String::new(),
    }
}

/// `normalizeContentForLock` + `jsonEqual`: equal content, ignoring the
/// stamped `media.attachmentId` (serde_json maps compare key-order free).
pub fn content_changed(stored: &Value, next: &Value) -> bool {
    fn normalize(content: &Value) -> Value {
        let mut out = content.clone();
        if let Some(media) = out.get_mut("media").and_then(Value::as_object_mut) {
            media.remove("attachmentId");
        }
        out
    }
    normalize(stored) != normalize(next)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_and_builds_a_button_message() {
        let content = json!({
            "templateType": "button",
            "text": " Hi ",
            "buttons": [
                { "id": "1", "title": "Go", "actionType": "openWebsite", "url": "https://x.io", "browserSize": 70 },
                { "id": "2", "title": "Flow", "actionType": "startExternalNode", "flowId": "5", "nodeId": "9" },
            ],
            "quickReplies": [{ "id": "3", "contentType": "user_email" }],
        });
        let parsed = parse(&content).expect("valid");
        assert_eq!(parsed["text"], "Hi");
        let payload = build_payload(&parsed);
        assert_eq!(
            payload["attachment"]["payload"]["buttons"][0]["webview_height_ratio"],
            "tall"
        );
        assert_eq!(
            payload["attachment"]["payload"]["buttons"][1]["payload"],
            "5:::::::9"
        );
        assert_eq!(
            payload["quick_replies"],
            json!([{ "content_type": "user_email" }])
        );
        assert_eq!(summarize(&parsed), "Hi");
        assert!(
            parse(&json!({ "templateType": "text", "text": "", "quickReplies": [] })).is_none()
        );
    }

    #[test]
    fn the_lock_ignores_the_attachment_id() {
        let a = json!({ "templateType": "media", "media": { "url": "u", "attachmentId": "7" } });
        let b = json!({ "media": { "url": "u" }, "templateType": "media" });
        assert!(!content_changed(&a, &b));
        assert!(content_changed(
            &a,
            &json!({ "templateType": "media", "media": { "url": "v" } })
        ));
    }
}
