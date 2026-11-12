//! Port of `packages/flow-config/src/util.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn button_payload_schema = z::object(vec![("f", z::string().regex(r#"^\d+$"#, "").regex(r#"^\d+$"#, "").refine(is_within_signed_bigint_range, Some("Value exceeds the signed bigint range"), None)),
("fv", z::string().regex(r#"^\d+$"#, "").regex(r#"^\d+$"#, "").refine(is_within_signed_bigint_range, Some("Value exceeds the signed bigint range"), None).optional()),
("b", z::string().regex(r#"^\d+$"#, "").regex(r#"^\d+$"#, "").refine(is_within_signed_bigint_range, Some("Value exceeds the signed bigint range"), None).optional()),
("br", z::string().regex(r#"^\d+$"#, "").regex(r#"^\d+$"#, "").refine(is_within_signed_bigint_range, Some("Value exceeds the signed bigint range"), None).optional()),
("ss", z::string().regex(r#"^\d+$"#, "").regex(r#"^\d+$"#, "").refine(is_within_signed_bigint_range, Some("Value exceeds the signed bigint range"), None).optional()),
("cid", z::string().regex(r#"^\d+$"#, "").regex(r#"^\d+$"#, "").refine(is_within_signed_bigint_range, Some("Value exceeds the signed bigint range"), None).optional()),
("ca", z::string().regex(r#"^\d+$"#, "").regex(r#"^\d+$"#, "").refine(is_within_signed_bigint_range, Some("Value exceeds the signed bigint range"), None).optional()),
("n", z::string().regex(r#"^\d+$"#, "").regex(r#"^\d+$"#, "").refine(is_within_signed_bigint_range, Some("Value exceeds the signed bigint range"), None).optional())]).transform(button_payload_schema_transform));

pub const ZALO_POSTBACK_TEXT_PREFIX: &str = "postback_";

pub fn flow_action_target_types() -> &'static Value {
    static V: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(r#"{"button":"button","quickReply":"quickReply"}"#)
            .expect("valid JSON")
    });
    &V
}

/// `ButtonPayload` (decoded flow button payload).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ButtonPayload {
    pub flow_id: String,
    pub flow_version_id: Option<String>,
    pub button_id: Option<String>,
    pub broadcast_id: Option<String>,
    pub sequence_step_id: Option<String>,
    pub contact_inbox_id: Option<String>,
    pub comment_automation_id: Option<String>,
    pub node_id: Option<String>,
}

/// `FlowActionTarget`.
#[derive(Debug, Clone, PartialEq)]
pub struct FlowActionTarget {
    pub details: Value,
    pub node_id: Option<String>,
    pub target_type: &'static str,
}

const MAX_SIGNED_BIGINT: u64 = i64::MAX as u64;

/// `isWithinSignedBigintRange`: a digit string ≤ 2^63 − 1.
pub fn is_within_signed_bigint_range(value: &Value) -> bool {
    value.as_str().is_some_and(|s| {
        let digits = s.trim();
        !digits.is_empty()
            && digits.bytes().all(|b| b.is_ascii_digit())
            && digits.parse::<u64>().is_ok_and(|n| n <= MAX_SIGNED_BIGINT)
    })
}

fn button_payload_schema_transform(data: Value, _ctx: &mut RefineCtx) -> Value {
    let mut out = json!({ "flowId": data["f"] });
    for (short, long) in [
        ("fv", "flowVersionId"),
        ("b", "buttonId"),
        ("br", "broadcastId"),
        ("ss", "sequenceStepId"),
        ("cid", "contactInboxId"),
        ("ca", "commentAutomationId"),
        ("n", "nodeId"),
    ] {
        if truthy(data.get(short)) {
            out[long] = data[short].clone();
        }
    }
    out
}

impl ButtonPayload {
    fn from_parsed(value: &Value) -> ButtonPayload {
        let opt = |k: &str| value.get(k).and_then(Value::as_str).map(str::to_owned);
        ButtonPayload {
            flow_id: str_field(value, "flowId").to_owned(),
            flow_version_id: opt("flowVersionId"),
            button_id: opt("buttonId"),
            broadcast_id: opt("broadcastId"),
            sequence_step_id: opt("sequenceStepId"),
            contact_inbox_id: opt("contactInboxId"),
            comment_automation_id: opt("commentAutomationId"),
            node_id: opt("nodeId"),
        }
    }
}

/// `encodeButtonPayload(props)`: positional `f:fv:b:br:ss:cid:ca:n`, trailing
/// empties trimmed (at least `f:`).
pub fn encode_button_payload(props: &ButtonPayload) -> String {
    let mut parts: Vec<&str> = vec![
        &props.flow_id,
        props.flow_version_id.as_deref().unwrap_or(""),
        props.button_id.as_deref().unwrap_or(""),
        props.broadcast_id.as_deref().unwrap_or(""),
        props.sequence_step_id.as_deref().unwrap_or(""),
        props.contact_inbox_id.as_deref().unwrap_or(""),
        props.comment_automation_id.as_deref().unwrap_or(""),
        props.node_id.as_deref().unwrap_or(""),
    ];
    while parts.len() > 2 && parts.last() == Some(&"") {
        parts.pop();
    }
    parts.join(":")
}

fn atob(text: &str) -> Option<Vec<u8>> {
    let cleaned: Vec<u8> = text.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    let trimmed: &[u8] = {
        let mut end = cleaned.len();
        if cleaned.len() % 4 == 0 {
            while end > 0 && cleaned[end - 1] == b'=' && cleaned.len() - end < 2 {
                end -= 1;
            }
        }
        &cleaned[..end]
    };
    if trimmed.len() % 4 == 1 {
        return None;
    }
    let value = |b: u8| -> Option<u32> {
        Some(match b {
            b'A'..=b'Z' => u32::from(b - b'A'),
            b'a'..=b'z' => u32::from(b - b'a') + 26,
            b'0'..=b'9' => u32::from(b - b'0') + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        })
    };
    let mut out = Vec::new();
    for chunk in trimmed.chunks(4) {
        let mut n = 0u32;
        for (i, b) in chunk.iter().enumerate() {
            n |= value(*b)? << (18 - 6 * i);
        }
        let bytes = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        out.extend_from_slice(&bytes[..chunk.len() - 1]);
    }
    Some(out)
}

fn is_bare_flow_id(payload: &str) -> bool {
    (13..=19).contains(&payload.len())
        && payload.bytes().all(|b| b.is_ascii_digit())
        && payload.parse::<u64>().is_ok_and(|n| n <= MAX_SIGNED_BIGINT)
}

/// `decodeButtonPayload(payload, { allowBareFlowId })`: the payload, or `None`.
pub fn decode_button_payload(payload: &str, allow_bare_flow_id: bool) -> Option<ButtonPayload> {
    let schema = button_payload_schema();
    let input = if allow_bare_flow_id && is_bare_flow_id(payload) {
        json!({ "f": payload })
    } else if payload.contains(':') {
        let parts: Vec<&str> = payload.split(':').collect();
        let mut input = Map::new();
        for (index, key) in ["f", "fv", "b", "br", "ss", "cid", "ca", "n"]
            .iter()
            .enumerate()
        {
            match parts.get(index) {
                Some(part) if index == 0 => {
                    input.insert((*key).into(), json!(part));
                }
                Some(part) if !part.is_empty() => {
                    input.insert((*key).into(), json!(part));
                }
                _ => {}
            }
        }
        Value::Object(input)
    } else {
        // Legacy format: base64 + JSON.
        let bytes = atob(payload)?;
        let text: String = bytes.iter().map(|b| char::from(*b)).collect();
        serde_json::from_str(&text).ok()?
    };
    let parsed = schema.parse(&input).ok()?;
    Some(ButtonPayload::from_parsed(&parsed))
}

/// `extractMetadata(key, metadata)`.
pub fn extract_metadata(key: &str, metadata: Option<&Value>) -> Option<String> {
    metadata?
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// `isMagicLinkUrl(url)`: a `/r/<a>/<b>` path.
pub fn is_magic_link_url(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    let mut segments = parsed
        .path()
        .strip_prefix("/r/")
        .map(|rest| rest.splitn(2, '/'));
    match segments.as_mut() {
        Some(it) => {
            let first = it.next().unwrap_or("");
            let second = it.next().unwrap_or("");
            !first.is_empty() && !second.is_empty() && !second.starts_with('/')
        }
        None => false,
    }
}

/// `appendCodeToMagicLink(url, code)`: sets `?code=` on a magic link.
pub fn append_code_to_magic_link(url: &str, code: &str) -> String {
    if !is_magic_link_url(url) {
        return url.to_owned();
    }
    let Ok(mut parsed) = url::Url::parse(url) else {
        return url.to_owned();
    };
    let pairs: Vec<(String, String)> = parsed
        .query_pairs()
        .filter(|(k, _)| k != "code")
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    let had_code = parsed.query_pairs().any(|(k, _)| k == "code");
    let mut out = url::form_urlencoded::Serializer::new(String::new());
    let original: Vec<(String, String)> = parsed
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    if had_code {
        // `searchParams.set` replaces the first `code` in place and drops the rest.
        let mut replaced = false;
        for (k, v) in &original {
            if k == "code" {
                if !replaced {
                    out.append_pair("code", code);
                    replaced = true;
                }
            } else {
                out.append_pair(k, v);
            }
        }
    } else {
        for (k, v) in &pairs {
            out.append_pair(k, v);
        }
        out.append_pair("code", code);
    }
    parsed.set_query(Some(&out.finish()));
    parsed.to_string()
}

/// `getNodeFromButton(nodes, buttonId)`: (button, runtime node id).
pub fn get_node_from_button(nodes: &[Value], button_id: &str) -> (Option<Value>, Option<String>) {
    match find_button_in_nodes(nodes, button_id) {
        Some(found) => (Some(found.button), Some(found.runtime_node_id)),
        None => (None, None),
    }
}

/// `getNodeFromQuickReply(nodes, quickReplyId)`.
pub fn get_node_from_quick_reply(
    nodes: &[Value],
    quick_reply_id: &str,
) -> (Option<Value>, Option<String>) {
    for node in nodes {
        let Some(replies) = node["data"]["details"]
            .get("quickReplies")
            .and_then(Value::as_array)
        else {
            continue;
        };
        if let Some(reply) = replies.iter().find(|r| r["id"] == quick_reply_id) {
            return (Some(reply.clone()), node["id"].as_str().map(str::to_owned));
        }
    }
    (None, None)
}

/// `resolveFlowActionTarget(nodes, actionId)`: step buttons win a tie with
/// node-level quick replies.
pub fn resolve_flow_action_target(nodes: &[Value], action_id: &str) -> Option<FlowActionTarget> {
    if let (Some(button), node_id) = get_node_from_button(nodes, action_id) {
        return Some(FlowActionTarget {
            details: button,
            node_id,
            target_type: "button",
        });
    }
    if let (Some(reply), node_id) = get_node_from_quick_reply(nodes, action_id) {
        return Some(FlowActionTarget {
            details: reply,
            node_id,
            target_type: "quickReply",
        });
    }
    None
}
