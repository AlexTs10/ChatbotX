//! The rows the variable resolvers read (`@chatbotx.io/database/types`,
//! only the columns the variables use). Timestamps are epoch milliseconds
//! (a JS `Date`). `Default` gives the empty row the original tests build
//! with a partial object literal.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// `ContactModel.location`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ContactLocation {
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
}

/// `ContactModel`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ContactModel {
    pub id: String,
    pub workspace_id: String,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub email: Option<String>,
    pub phone_number: Option<String>,
    /// A storage key, an absolute URL or a no-avatar sentinel.
    pub avatar: Option<String>,
    pub gender: Option<String>,
    pub locale: Option<String>,
    pub timezone: Option<String>,
    pub country: Option<String>,
    pub state: Option<String>,
    pub city: Option<String>,
    /// `ref`.
    pub ref_: Option<String>,
    pub location: Option<ContactLocation>,
    /// `None` for a contact serialized before the column existed.
    pub total_tagged: Option<i64>,
    pub total_new_tagged: Option<i64>,
}

/// `ContactInboxModel.referral`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Referral {
    /// `ref`.
    pub ref_: Option<String>,
    pub ad_id: Option<String>,
    pub ad_title: Option<String>,
    pub ad_body: Option<String>,
    pub ctwa_clid: Option<String>,
    pub source_url: Option<String>,
    pub source_platform: Option<String>,
}

/// `ContactInboxModel`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ContactInboxModel {
    pub id: String,
    pub inbox_id: String,
    pub source_id: String,
    pub channel: String,
    pub source: Option<String>,
    pub created_at: Option<i64>,
    pub contact_last_read_at: Option<i64>,
    pub last_incoming_message_at: Option<i64>,
    pub last_outbound_message_at: Option<i64>,
    pub first_interaction_at: Option<i64>,
    pub language: Option<String>,
    pub last_btn_title: Option<String>,
    pub last_story_id: Option<String>,
    pub last_input_failure: Option<String>,
    pub last_error_log: Option<String>,
    pub webchat_parent_url: Option<String>,
    pub consecutive_failed_reply: Option<i64>,
    pub last_comment_message_id: Option<String>,
    pub last_comment_message_at: Option<i64>,
    pub referral: Option<Referral>,
    pub source_username: Option<String>,
    pub source_user_id: Option<String>,
    pub follows_business: Option<bool>,
    pub business_follows_contact: Option<bool>,
    pub account_verified: Option<bool>,
    pub follower_count: Option<i64>,
}

/// `WorkspaceModel`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WorkspaceModel {
    pub id: String,
    pub name: Option<String>,
    pub logo: Option<String>,
    pub timezone: Option<String>,
    pub language: Option<String>,
    pub scheduled_deletion_at: Option<i64>,
}

/// `ConversationModel`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConversationModel {
    pub id: String,
    pub workspace_id: String,
    pub contact_id: String,
    pub assigned_user_id: Option<String>,
    pub assigned_inbox_team_id: Option<String>,
    pub last_step: Option<String>,
    pub current_step: Option<String>,
    pub last_activity_at: Option<i64>,
    pub created_at: Option<i64>,
}

/// The `user` of `workspaceMemberService.findWithUserByWorkspaceIdAndUserId`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemberUser {
    pub id: String,
    pub name: Option<String>,
    pub email: Option<String>,
}

/// `MessageModel` (a comment the contact wrote).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MessageModel {
    pub id: String,
    pub source_id: Option<String>,
    pub text: Option<String>,
    pub content_attributes: Option<serde_json::Value>,
    pub deleted_at: Option<i64>,
    pub created_at: Option<i64>,
}

/// One row of `messageService.listLastMessages`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChatMessage {
    pub text: Option<String>,
    /// `senderTypes`: `bot`, `contact`, `system`, `user`, `api`.
    pub sender_type: String,
}

/// An attachment of [`MessageWithAttachments`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Attachment {
    pub id: String,
    pub file_type: String,
    pub origin_path: String,
    pub message_created_at: Option<i64>,
}

/// `MessageWithAttachments`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MessageWithAttachments {
    pub contact_inbox_id: String,
    /// `contentTypes`: `text`, `location`, `refLink`.
    pub content_type: String,
    pub text: Option<String>,
    pub attachments: Vec<Attachment>,
}

/// A WhatsApp call row (`whatsappCallRepository`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CallRecord {
    pub id: String,
    pub recording_path: Option<String>,
    pub transcript: Option<String>,
}

/// `resolveTenantSettings`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TenantSettings {
    pub app_url: String,
    pub storage_url: String,
}

/// One integration row of an inbox (only what the variables read).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct IntegrationRow {
    pub id: String,
    pub name: Option<String>,
    pub page_id: Option<String>,
    /// The provider credentials, handed back to the provider calls.
    pub auth: serde_json::Value,
}

/// `InboxWithIntegrations`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct InboxWithIntegrations {
    pub id: String,
    pub workspace_id: String,
    pub integration_messenger: Option<IntegrationRow>,
    pub integration_instagram: Option<IntegrationRow>,
    pub integration_whatsapp: Option<IntegrationRow>,
    pub integration_zalo: Option<IntegrationRow>,
    pub integration_tiktok: Option<IntegrationRow>,
    pub integration_threads: Option<IntegrationRow>,
    pub integration_telegram: Option<IntegrationRow>,
    pub integration_webchat: Option<IntegrationRow>,
    pub integration_smtp: Option<IntegrationRow>,
}

impl InboxWithIntegrations {
    /// The `integration*` relations of `inboxRelations`, as
    /// `(channel, relation name)`: the channels an inbox can carry an
    /// integration row for.
    pub const INTEGRATION_RELATIONS: &'static [(&'static str, &'static str)] = &[
        ("messenger", "integrationMessenger"),
        ("webchat", "integrationWebchat"),
        ("zalo", "integrationZalo"),
        ("whatsapp", "integrationWhatsapp"),
        ("telegram", "integrationTelegram"),
        ("smtp", "integrationSmtp"),
        ("tiktok", "integrationTiktok"),
        ("instagram", "integrationInstagram"),
        ("threads", "integrationThreads"),
    ];

    /// The integration row of `channel` (`getChannelIntegrationId`'s
    /// switch); `None` for a channel without one.
    pub fn integration(&self, channel: &str) -> Option<&IntegrationRow> {
        match channel {
            "instagram" => self.integration_instagram.as_ref(),
            "messenger" => self.integration_messenger.as_ref(),
            "whatsapp" => self.integration_whatsapp.as_ref(),
            "zalo" => self.integration_zalo.as_ref(),
            "tiktok" => self.integration_tiktok.as_ref(),
            "threads" => self.integration_threads.as_ref(),
            "telegram" => self.integration_telegram.as_ref(),
            "webchat" => self.integration_webchat.as_ref(),
            "smtp" => self.integration_smtp.as_ref(),
            _ => None,
        }
    }

    /// Sets the integration row of `channel` (test fixtures and adapters).
    /// Returns `false` for a channel without an integration relation.
    pub fn set_integration(&mut self, channel: &str, row: IntegrationRow) -> bool {
        let slot = match channel {
            "instagram" => &mut self.integration_instagram,
            "messenger" => &mut self.integration_messenger,
            "whatsapp" => &mut self.integration_whatsapp,
            "zalo" => &mut self.integration_zalo,
            "tiktok" => &mut self.integration_tiktok,
            "threads" => &mut self.integration_threads,
            "telegram" => &mut self.integration_telegram,
            "webchat" => &mut self.integration_webchat,
            "smtp" => &mut self.integration_smtp,
            _ => return false,
        };
        *slot = Some(row);
        true
    }
}

/// `ContactProfileSnapshot` (`@chatbotx.io/sdk`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContactProfileSnapshot {
    pub follows_business: Option<bool>,
    pub business_follows_contact: Option<bool>,
    pub account_verified: Option<bool>,
    pub follower_count: Option<i64>,
    pub username: Option<String>,
}

/// An appointment with its calendar name (`appointmentService.findBy`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Appointment {
    pub id: String,
    pub workspace_id: String,
    pub contact_id: String,
    pub conversation_id: Option<String>,
    pub start_at: Option<i64>,
    pub invitee_timezone: Option<String>,
    /// `calendar.name`.
    pub calendar_name: Option<String>,
}

/// A Facebook lead-ads lead (`facebookLeadAdsLeadService.findLatestByContact`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LeadAdsLead {
    pub leadgen_id: Option<String>,
    pub form_id: Option<String>,
    pub form_name: Option<String>,
    pub adgroup_id: Option<String>,
}

/// `PipelineVariableTarget`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PipelineVariableTarget {
    pub id: String,
    pub reference_name: String,
    pub name: String,
    pub stage_name: String,
    pub status: String,
    pub status_label: String,
    pub priority: String,
    pub value: Option<f64>,
    pub currency: String,
    pub assigned_admins: Vec<String>,
    pub created_at: i64,
    pub due_date: Option<i64>,
    pub updated_at: i64,
}

/// `ContactCustomFieldValue`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContactCustomFieldValue {
    pub key: String,
    /// `CustomFieldType`.
    pub kind: String,
    /// `None` where a corrupt row carries no value.
    pub value: Option<String>,
    pub description: String,
}

/// `BotFieldValue`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BotFieldValue {
    /// `type`.
    #[serde(rename = "type")]
    pub kind: String,
    pub value: Option<String>,
}

/// A `BotField` row.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BotFieldRow {
    pub id: String,
    pub kind: String,
    pub value: Option<String>,
}

/// The bot fields of a workspace, keyed by id.
pub type BotFieldsMap = HashMap<String, BotFieldValue>;

/// The custom field values of a contact, keyed by field name.
pub type CustomFieldsMap = HashMap<String, ContactCustomFieldValue>;
