//! The `whatsappVoipSignaling` queue (`packages/worker-config/src/queues/whatsapp-voip-signaling`):
//! job actions, payloads and job ids.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use super::heavy::SchemaError;

/// `WhatsappVoipSignalingJobAction`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WhatsappVoipSignalingJobAction {
    HandleConnect,
    ExpireIfUnanswered,
    HandleOutboundAnswer,
    ExpireOutboundDial,
}

impl WhatsappVoipSignalingJobAction {
    /// `"handleConnect"`, `"expireIfUnanswered"`, `"handleOutboundAnswer"`, `"expireOutboundDial"`.
    pub fn as_str(self) -> &'static str {
        todo!()
    }
}

/// `handleOutboundAnswer` data: never an sdp or session; `wacid` may be
/// absent (answer/connect race).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HandleOutboundAnswerData {
    pub attempt_id: String,
    pub whatsapp_call_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wacid: Option<String>,
    pub workspace_id: String,
}

/// `expireOutboundDial` data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExpireOutboundDialData {
    pub attempt_id: String,
    pub whatsapp_call_id: String,
    pub workspace_id: String,
    /// Unix ms.
    pub deadline_at: i64,
}

/// `WhatsappVoipSignalingJobData`. The inbound payloads are not pinned by
/// the frozen tests and stay JSON here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum WhatsappVoipSignalingJobData {
    #[serde(rename = "handleConnect")]
    HandleConnect(Value),
    #[serde(rename = "expireIfUnanswered")]
    ExpireIfUnanswered(Value),
    #[serde(rename = "handleOutboundAnswer")]
    HandleOutboundAnswer(HandleOutboundAnswerData),
    #[serde(rename = "expireOutboundDial")]
    ExpireOutboundDial(ExpireOutboundDialData),
}

impl WhatsappVoipSignalingJobData {
    /// Parse a `{ type, data }` job payload.
    pub fn parse(value: &Value) -> Result<WhatsappVoipSignalingJobData, SchemaError> {
        todo!()
    }

    /// The JSON written to the queue.
    pub fn to_value(&self) -> Value {
        todo!()
    }
}

/// `whatsappVoipSignalingJobId(wacid)` (inbound connect).
pub fn whatsapp_voip_signaling_job_id(wacid: &str) -> String {
    todo!()
}

/// `whatsappVoipExpiryJobId(wacid)` (inbound expiry).
pub fn whatsapp_voip_expiry_job_id(wacid: &str) -> String {
    todo!()
}

/// `outboundAnswerJobId(attemptId)` = `voip-out-answer-<attemptId>`.
pub fn outbound_answer_job_id(attempt_id: &str) -> String {
    todo!()
}

/// `expireOutboundDialJobId(attemptId)` = `voip-out-expire-<attemptId>`.
pub fn expire_outbound_dial_job_id(attempt_id: &str) -> String {
    todo!()
}
