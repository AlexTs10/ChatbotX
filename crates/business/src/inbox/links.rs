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
    REF_CAPABLE_CHANNELS.contains(&channel)
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
    let ref_value = ref_config.map(encode_ref);
    let (base, key, value) = match inbox.channel.as_str() {
        "messenger" => (
            format!("https://m.me/{}", inbox.source_id),
            "ref",
            ref_value,
        ),
        "instagram" => (
            format!(
                "https://ig.me/m/{}",
                inbox.instagram_username.as_deref().unwrap_or("undefined")
            ),
            "ref",
            ref_value,
        ),
        "whatsapp" => (
            format!(
                "https://wa.me/{}",
                inbox.whatsapp_display_phone_number.as_deref().unwrap_or("")
            ),
            "text",
            ref_value.map(|v| format!("/ref-{v}")),
        ),
        "telegram" => (format!("https://t.me/{}", inbox.name), "start", ref_value),
        "zalo" => (
            format!("https://zalo.me/{}", inbox.source_id),
            "ref",
            ref_value,
        ),
        "webchat" => (
            format!(
                "{app_url}/webchat?workspaceId={}&webchatId={}",
                inbox.workspace_id, inbox.source_id
            ),
            "ref",
            ref_value,
        ),
        "tiktok" => (
            format!("https://tiktok.me/{}", inbox.source_id),
            "ref",
            ref_value,
        ),
        _ => return None,
    };
    let mut url = Url::parse(&base).ok()?;
    if let Some(value) = value.filter(|v| !v.is_empty()) {
        // `url.searchParams.set(key, value)`: drop the key, append it, re-serialize.
        let mut pairs: Vec<(String, String)> = url
            .query_pairs()
            .filter(|(k, _)| k != key)
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        pairs.push((key.to_string(), value));
        url.query_pairs_mut().clear().extend_pairs(pairs);
    }
    Some(url.to_string())
}

/// `getInboxLinks`: the inboxes that have a link, with it.
pub fn get_inbox_links(
    app_url: &str,
    inboxes: &[LinkInbox],
    ref_config: Option<&RefConfig>,
) -> Vec<(LinkInbox, String)> {
    inboxes
        .iter()
        .filter_map(|inbox| {
            build_inbox_link(app_url, inbox, ref_config).map(|url| (inbox.clone(), url))
        })
        .collect()
}

/// `inboxService.listAllConnectedByWorkspace({ includes: ["integration"] })`.
pub fn list_connected_link_inboxes(conn: &Connection, workspace_id: i64) -> Result<Vec<LinkInbox>> {
    let mut stmt = conn
        .prepare_cached(
            "SELECT i.\"id\", i.\"workspaceId\", i.\"name\", i.\"channel\", i.\"sourceId\", ig.\"username\", wa.\"displayPhoneNumber\" \
             FROM \"Inbox\" i \
             LEFT JOIN \"IntegrationInstagram\" ig ON ig.\"inboxId\" = i.\"id\" \
             LEFT JOIN \"IntegrationWhatsapp\" wa ON wa.\"inboxId\" = i.\"id\" \
             WHERE i.\"workspaceId\" = ?1 AND i.\"status\" = 'connected' ORDER BY i.\"id\"",
        )
        .map_err(chatbotx_db::repo::Error::from)?;
    let rows = stmt
        .query_map([workspace_id], |r| {
            Ok(LinkInbox {
                id: r.get(0)?,
                workspace_id: r.get(1)?,
                name: r.get(2)?,
                channel: r.get(3)?,
                source_id: r.get(4)?,
                instagram_username: r.get(5)?,
                whatsapp_display_phone_number: r.get(6)?,
            })
        })
        .map_err(chatbotx_db::repo::Error::from)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(chatbotx_db::repo::Error::from)?;
    Ok(rows)
}
