//! `inbox/utils.ts` (`REF_CAPABLE_CHANNELS`, `buildInboxLink`, `getInboxLinks`)
//! and `inboxService.listAllConnectedByWorkspace` with its integrations, as a
//! private query (moves to the inbox port later).

use chatbotx_db::rusqlite::Connection;
use url::Url;

use crate::errors::Result;
use crate::referral::{RefConfig, encode_ref};

/// `REF_CAPABLE_CHANNELS`: channels whose incoming handler reads a ref.
pub const REF_CAPABLE_CHANNELS: &[&str] =
    &["messenger", "instagram", "whatsapp", "telegram", "webchat"];

/// `canReceiveRef`.
pub fn can_receive_ref(channel: &str) -> bool {
    todo!("P3.6-D1")
}

/// An inbox with the integration fields its link needs (`InboxWithIntegrations`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkInbox {
    pub id: i64,
    pub workspace_id: i64,
    pub name: String,
    pub channel: String,
    pub source_id: String,
    /// `integrationInstagram.username`.
    pub instagram_username: Option<String>,
    /// `integrationWhatsapp.displayPhoneNumber`.
    pub whatsapp_display_phone_number: Option<String>,
}

/// `buildInboxLink`: the chat link of an inbox, with the ref when given.
pub fn build_inbox_link(
    app_url: &str,
    inbox: &LinkInbox,
    ref_config: Option<&RefConfig>,
) -> Option<String> {
    todo!("P3.6-D1")
}

/// `getInboxLinks`: the inboxes that have a link, with it.
pub fn get_inbox_links(
    app_url: &str,
    inboxes: &[LinkInbox],
    ref_config: Option<&RefConfig>,
) -> Vec<(LinkInbox, String)> {
    todo!("P3.6-D1")
}

/// `inboxService.listAllConnectedByWorkspace({ includes: ["integration"] })`.
pub fn list_connected_link_inboxes(conn: &Connection, workspace_id: i64) -> Result<Vec<LinkInbox>> {
    todo!("P3.6-D1")
}
