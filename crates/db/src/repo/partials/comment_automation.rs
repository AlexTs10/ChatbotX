//! Port of the reply-text helpers of `partials/comment-automation.ts`.

use serde::{Deserialize, Serialize};

/// `COMMENT_REPLY_MAX_TEXTS`.
pub const COMMENT_REPLY_MAX_TEXTS: usize = 10;

/// `commentReplyTypes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommentReplyType {
    #[serde(rename = "AIAgent")]
    AiAgent,
    #[serde(rename = "text")]
    Text,
    #[serde(rename = "flow")]
    Flow,
    #[serde(rename = "none")]
    None,
}

/// One message of a `text` reply (`{ value }`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommentReplyText {
    pub value: String,
}

/// `commentReplySchema`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommentReply {
    #[serde(rename = "type")]
    pub type_: CommentReplyType,
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub values: Option<Vec<CommentReplyText>>,
}

/// `resolveReplyTexts`: `values` (else the legacy `value`), trimmed, blanks dropped.
pub fn resolve_reply_texts(reply: &CommentReply) -> Vec<String> {
    let texts: Vec<&str> = match &reply.values {
        Some(values) => values.iter().map(|item| item.value.as_str()).collect(),
        None => vec![reply.value.as_deref().unwrap_or("")],
    };
    texts
        .into_iter()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
        .collect()
}

/// `normalizeReplyTexts`: a `text` reply gets `value` = the first message and `values`
/// built from a legacy `value`; other replies are returned as they are.
pub fn normalize_reply_texts(reply: CommentReply) -> CommentReply {
    if reply.type_ != CommentReplyType::Text {
        return reply;
    }
    match reply.values {
        Some(values) => {
            let first = values
                .first()
                .map(|item| item.value.clone())
                .unwrap_or_default();
            CommentReply {
                value: Some(first),
                values: Some(values),
                ..reply
            }
        }
        None => {
            let value = reply.value.clone().unwrap_or_default();
            CommentReply {
                values: Some(vec![CommentReplyText { value }]),
                ..reply
            }
        }
    }
}
