//! The `whatsappIdentityChange` integration job (typed normalized payload).

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `IntegrationJobAction.whatsappIdentityChange`.
pub const WHATSAPP_IDENTITY_CHANGE_ACTION: &str = "whatsappIdentityChange";

/// The normalized identity change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum WhatsappIdentityChangeKind {
    #[serde(rename_all = "camelCase")]
    UserIdChanged {
        user_id: String,
        previous_phone: String,
        new_phone: String,
    },
}

/// `payload` of the job.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WhatsappIdentityChangePayload {
    pub phone_number_id: String,
    pub message_id: String,
    pub change: WhatsappIdentityChangeKind,
}

/// `data` of the job.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WhatsappIdentityChangeData {
    pub integration_type: String,
    pub integration_identifier: String,
    pub payload: WhatsappIdentityChangePayload,
}

/// `IntegrationJobWhatsappIdentityChange`.
#[derive(Debug, Clone, PartialEq)]
pub struct WhatsappIdentityChangeJob {
    pub data: WhatsappIdentityChangeData,
}

impl WhatsappIdentityChangeJob {
    /// The job type (`"whatsappIdentityChange"`).
    pub fn job_type(&self) -> &'static str {
        WHATSAPP_IDENTITY_CHANGE_ACTION
    }

    /// `{ type, data }` as written to the queue.
    pub fn to_value(&self) -> Value {
        serde_json::json!({
            "type": self.job_type(),
            "data": serde_json::to_value(&self.data).unwrap_or(Value::Null),
        })
    }
}
