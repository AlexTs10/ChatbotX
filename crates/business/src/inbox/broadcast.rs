//! `inboxService.resolveBroadcastInboxIds` (`packages/business/src/inbox/service.ts`):
//! the inbox ids an audience-style key means. Strategies in order: an
//! explicit `inboxIds` list (narrowed to the workspace and the channels),
//! the legacy WhatsApp / Messenger integration columns, then the channels.
//! Private queries on `"Inbox"`, `"IntegrationWhatsapp"`, `"IntegrationMessenger"`.

use chatbotx_db::repo;
use chatbotx_db::rusqlite::Connection;

/// `BroadcastInboxResolutionInput`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BroadcastInboxResolutionInput {
    pub workspace_id: i64,
    pub channels: Option<Vec<String>>,
    pub inbox_ids: Option<Vec<i64>>,
    pub integration_whatsapp_id: Option<i64>,
    pub integration_messenger_id: Option<i64>,
}

/// `resolveBroadcastInboxIds`.
pub fn resolve_broadcast_inbox_ids(
    conn: &Connection,
    input: &BroadcastInboxResolutionInput,
) -> repo::Result<Vec<i64>> {
    let _ = (conn, input);
    todo!("P3.6-D2")
}
