//! `media/resolve-media-url.ts` `resolveContactAvatarUrl` (the public
//! read, `publicUrls: true`): a stored key under the tenant storage URL, the
//! default placeholder for a fresh no-avatar sentinel, a signed
//! `/media/avatar/<token>` proxy URL for a Messenger / Instagram contact
//! without an avatar.

/// One contact inbox of the avatar choice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvatarInbox {
    pub id: i64,
    pub channel: String,
    pub last_message_at: Option<i64>,
}

/// The tenant URLs (`appUrl`, `storageUrl` with a trailing `/`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TenantUrls {
    pub app_url: String,
    pub storage_url: String,
}

impl TenantUrls {
    /// `deriveUrls`: `NEXT_PUBLIC_STORAGE_URL` (+ `/`) or `<appUrl>/storage/`.
    pub fn from_app_url(app_url: &str, storage_base_url: Option<&str>) -> TenantUrls {
        let _ = (app_url, storage_base_url);
        todo!("P3.6-D2")
    }
}

/// `resolveContactAvatarUrl` at `now_ms`.
pub fn resolve_contact_avatar_url(
    workspace_id: i64,
    avatar: Option<&str>,
    contact_inboxes: &[AvatarInbox],
    urls: &TenantUrls,
    now_ms: i64,
) -> Option<String> {
    let _ = (workspace_id, avatar, contact_inboxes, urls, now_ms);
    todo!("P3.6-D2")
}
