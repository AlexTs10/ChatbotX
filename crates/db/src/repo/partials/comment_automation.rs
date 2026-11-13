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
    let _ = reply;
    todo!("P3.2b")
}

/// `normalizeReplyTexts`: a `text` reply gets `value` = the first message and `values`
/// built from a legacy `value`; other replies are returned as they are.
pub fn normalize_reply_texts(reply: CommentReply) -> CommentReply {
    let _ = reply;
    todo!("P3.2b")
}
