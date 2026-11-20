//! The Messenger Ads JSON of a flow's start node
//! (`integrations/messenger/src/messenger-ads-json.ts`
//! `convertStartNodeToMessengerAdsJson`) and the published start node it
//! reads (`flowVersionService.getMessengerAdsStartNode`, without the cache).

use std::cell::Cell;
use std::sync::OnceLock;

use chatbotx_core::flow_config::send_message_node_schema;
use chatbotx_core::flow_config::util::{
    ButtonPayload, append_code_to_magic_link, encode_button_payload,
};
use chatbotx_core::utils::variables::contains_variable_placeholder;
use chatbotx_db::Db;
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use regex::Regex;
use serde_json::{Map, Value, json};

use crate::errors::Result;
use crate::tx;

/// `ALLOWED_STEP_TYPES` (`sendCard` lives only inside a carousel).
const ALLOWED_STEP_TYPES: &[&str] = &[
    "sendText",
    "sendImage",
    "sendMultipleImages",
    "sendCarousel",
    "sendVideo",
    "sendAudio",
    "sendFile",
];
/// `ALLOWED_VARIABLES`.
const ALLOWED_VARIABLES: &[&str] = &["first_name", "last_name", "full_name"];
const MAX_QUICK_REPLIES: usize = 13;
const MAX_CAROUSEL_ELEMENTS: usize = 10;
/// `MAX_BUTTONS` of the Messenger integration.
const MAX_BUTTONS: usize = 3;

/// `MessengerAdsJsonMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// The copy-JSON dialog: any unsupported step / variable is an error.
    Strict,
    /// The welcome message flow sync: skipped, blanked, always `Ok`.
    Lenient,
}

/// `MessengerAdsJsonResult`: the `{ message }` array, or the reason
/// (`invalidStepType` / `invalidVariable`).
pub type AdsJson = std::result::Result<Vec<Value>, &'static str>;

/// `VARIABLE_REGEX`.
fn variable_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\{\{\s*([^}]+?)\s*\}\}").expect("valid regex"))
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or_default()
}

/// `withLenientSteps`: the raw node without the steps lenient mode skips.
fn with_lenient_steps(node: &Value) -> Value {
    let mut node = node.clone();
    if let Some(steps) = node["data"]["details"]["steps"].as_array_mut() {
        steps.retain(|step| {
            let kind = text(step, "stepType");
            ALLOWED_STEP_TYPES.contains(&kind) || kind == "sendGif"
        });
    }
    node
}

/// `mediaType`.
fn media_type(step_type: &str) -> &'static str {
    match step_type {
        "sendImage" | "sendGif" => "image",
        "sendVideo" => "video",
        "sendAudio" => "audio",
        _ => "file",
    }
}

/// `getAttachmentTemplate`.
fn attachment(url: &str, kind: &str) -> Value {
    json!({ "type": kind, "payload": { "url": url, "is_reusable": true } })
}

/// `convertStartNodeToMessengerAdsJson`.
pub fn convert(
    start_node: &Value,
    flow_id: &str,
    flow_version_id: Option<&str>,
    mode: Mode,
) -> AdsJson {
    let lenient = mode == Mode::Lenient;
    let input = if lenient {
        with_lenient_steps(start_node)
    } else {
        start_node.clone()
    };
    let parsed = send_message_node_schema().safe_parse(&input);
    let Some(node) = parsed.data.filter(|_| parsed.success) else {
        // Not a Send Message node: no message content to offer.
        return if lenient {
            Ok(Vec::new())
        } else {
            Err("invalidStepType")
        };
    };
    let details = &node["data"]["details"];
    let steps = details["steps"].as_array().cloned().unwrap_or_default();
    if !lenient
        && steps
            .iter()
            .any(|s| !ALLOWED_STEP_TYPES.contains(&text(s, "stepType")))
    {
        return Err("invalidStepType");
    }

    let invalid = Cell::new(false);
    // Rewrites the 3 allowed variables and flags any other token.
    let rewrite = |value: &str| -> String {
        variable_re()
            .replace_all(value, |caps: &regex::Captures<'_>| {
                let name = caps[1].trim();
                if ALLOWED_VARIABLES.contains(&name) {
                    format!("{{{{user_{name}}}}}")
                } else {
                    invalid.set(true);
                    if lenient {
                        String::new()
                    } else {
                        format!("{{{{{name}}}}}")
                    }
                }
            })
            .into_owned()
    };
    let rewrite_label = |label: &str| -> String {
        if lenient {
            rewrite(label).trim().to_string()
        } else {
            rewrite(label)
        }
    };
    let is_blank_label = |label: &str| lenient && rewrite_label(label).is_empty();
    let payload = |button_id: &str| {
        encode_button_payload(&ButtonPayload {
            flow_id: flow_id.to_owned(),
            flow_version_id: flow_version_id.map(str::to_owned),
            button_id: Some(button_id.to_owned()),
            ..ButtonPayload::default()
        })
    };
    // `shouldDropButton` + `convertButtons` (`getButtonTemplate`, the first
    // `MAX_BUTTONS`); `None` when no button is left.
    let build_buttons = |buttons: &Value| -> Option<Vec<Value>> {
        let kept: Vec<&Value> = buttons
            .as_array()
            .into_iter()
            .flatten()
            .filter(|button| {
                let call = text(button, "buttonType") == "callPhoneNumber";
                if call && contains_variable_placeholder(text(&button["beforeStep"], "phoneNumber"))
                {
                    invalid.set(true);
                    return !lenient;
                }
                !is_blank_label(text(button, "label"))
            })
            .take(MAX_BUTTONS)
            .collect();
        if kept.is_empty() {
            return None;
        }
        Some(
            kept.into_iter()
                .map(|button| {
                    let title = rewrite_label(text(button, "label"));
                    let tapped = payload(text(button, "id"));
                    let before = &button["beforeStep"];
                    match text(button, "buttonType") {
                        "openWebsite" => json!({
                            "type": "web_url",
                            "title": title,
                            "url": append_code_to_magic_link(text(before, "url"), &tapped),
                        }),
                        "callPhoneNumber" => json!({
                            "type": "phone_number",
                            "title": title,
                            "payload": text(before, "phoneNumber"),
                        }),
                        _ => json!({ "type": "postback", "title": title, "payload": tapped }),
                    }
                })
                .collect(),
        )
    };

    let mut messages: Vec<Value> = Vec::new();
    for step in &steps {
        let step_type = text(step, "stepType");
        match step_type {
            "sendText" => {
                let body = rewrite(text(step, "text"));
                // Meta rejects an empty text; lenient mode drops the step.
                if lenient && body.trim().is_empty() {
                    continue;
                }
                messages.push(match build_buttons(&step["buttons"]) {
                    Some(buttons) => {
                        json!({ "message": { "attachment": { "type": "template", "payload": {
                        "template_type": "button", "text": body, "buttons": buttons,
                    } } } })
                    }
                    None => json!({ "message": { "text": body } }),
                });
            }
            "sendCarousel" => {
                let cards: Vec<&Value> = step["cards"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|card| !is_blank_label(text(card, "title")))
                    .collect();
                for group in cards.chunks(MAX_CAROUSEL_ELEMENTS) {
                    let elements: Vec<Value> = group
                        .iter()
                        .map(|card| {
                            let mut element = Map::new();
                            element
                                .insert("title".into(), json!(rewrite_label(text(card, "title"))));
                            let subtitle = text(card, "subtitle");
                            if !subtitle.is_empty() {
                                let subtitle = rewrite_label(subtitle);
                                if !subtitle.is_empty() {
                                    element.insert("subtitle".into(), json!(subtitle));
                                }
                            }
                            let image = text(&card["image"], "url");
                            if !image.is_empty() {
                                element.insert("image_url".into(), json!(image));
                            }
                            if let Some(buttons) = build_buttons(&card["buttons"]) {
                                element.insert("buttons".into(), json!(buttons));
                            }
                            Value::Object(element)
                        })
                        .collect();
                    messages.push(
                        json!({ "message": { "attachment": { "type": "template", "payload": {
                        "template_type": "generic", "elements": elements,
                    } } } }),
                    );
                }
            }
            "sendImage" | "sendVideo" | "sendAudio" | "sendFile" | "sendGif" => {
                messages.push(json!({ "message": {
                    "attachment": attachment(text(step, "url"), media_type(step_type)),
                } }));
            }
            "sendMultipleImages" => {
                let attachments: Vec<Value> = step["images"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|image| attachment(text(image, "url"), "image"))
                    .collect();
                messages.push(json!({ "message": { "attachments": attachments } }));
            }
            _ => {}
        }
    }

    // Node-level quick replies ride on the last message.
    let quick_replies: Vec<Value> = details["quickReplies"]
        .as_array()
        .into_iter()
        .flatten()
        // `qr.buttonType !== null` (a missing key passes).
        .filter(|qr| {
            qr.get("buttonType") != Some(&Value::Null) && !is_blank_label(text(qr, "label"))
        })
        .take(MAX_QUICK_REPLIES)
        .map(|qr| {
            json!({
                "content_type": "text",
                "title": rewrite_label(text(qr, "label")),
                "payload": payload(text(qr, "id")),
            })
        })
        .collect();
    if let Some(last) = messages.last_mut()
        && !quick_replies.is_empty()
    {
        last["message"]["quick_replies"] = json!(quick_replies);
    }

    if invalid.get() && !lenient {
        return Err("invalidVariable");
    }
    Ok(messages)
}

/// `getMessengerAdsStartNode`.
#[derive(Debug, Clone, PartialEq)]
pub enum StartNode {
    Ok {
        flow_version_id: i64,
        start_node: Value,
    },
    NotPublished,
    NoStartNode,
}

/// The latest published version's start node: the node flagged
/// `data.isStartNode`, else the `startNodeId` column (which is not re-synced
/// when the start node is reassigned).
pub async fn start_node(db: &Db, workspace_id: i64, flow_id: i64) -> Result<StartNode> {
    let row: Option<(i64, String, i64)> = tx::read(db, move |conn| {
        Ok(conn
            .query_row(
                r#"SELECT "id", "nodes", "startNodeId" FROM "FlowVersion"
                   WHERE "flowId" = ?1 AND "workspaceId" = ?2 AND "isDraft" = 0 AND "isLatest" = 1
                   LIMIT 1"#,
                params![flow_id, workspace_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?)
    })
    .await?;
    let Some((id, nodes, start_node_id)) = row else {
        return Ok(StartNode::NotPublished);
    };
    let nodes: Vec<Value> = serde_json::from_str(&nodes).unwrap_or_default();
    let start_id = start_node_id.to_string();
    let found = nodes
        .iter()
        .find(|node| node["data"]["isStartNode"] == true)
        .or_else(|| {
            nodes
                .iter()
                .find(|node| node["id"].as_str() == Some(start_id.as_str()))
        });
    Ok(match found {
        Some(node) => StartNode::Ok {
            flow_version_id: id,
            start_node: node.clone(),
        },
        None => StartNode::NoStartNode,
    })
}
