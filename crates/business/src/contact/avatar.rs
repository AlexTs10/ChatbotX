//! `media/resolve-media-url.ts` `resolveContactAvatarUrl` (the public
//! read, `publicUrls: true`): a stored key under the tenant storage URL, the
//! default placeholder for a fresh no-avatar sentinel, a signed
//! `/media/avatar/<token>` proxy URL for a Messenger / Instagram contact
//! without an avatar.

use std::sync::OnceLock;

use chatbotx_core::encryption::{
    Encryption, Keys, MEDIA_TOKEN_TTL_MS, MediaTokenClaims, MediaTokenKind,
};
use chatbotx_core::env::ProcessEnv;
use chatbotx_core::utils::no_avatar_sentinel::{
    DEFAULT_AVATAR_PLACEHOLDER_PATH, has_real_avatar, is_no_avatar_sentinel_fresh,
    parse_no_avatar_sentinel,
};
use chatbotx_core::variables::helpers::public_storage_url;

/// `AVATAR_HYDRATION_CHANNELS`.
const AVATAR_HYDRATION_CHANNELS: &[&str] = &["messenger", "instagram"];

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
        let app_url = app_url.trim_end_matches('/').to_string();
        let storage_url = match storage_base_url.filter(|url| !url.is_empty()) {
            Some(base) => format!("{}/", base.trim_end_matches('/')),
            None => format!("{app_url}/storage/"),
        };
        TenantUrls {
            app_url,
            storage_url,
        }
    }
}

/// The process media-token signer: `ENCRYPTION_KEY` (+ id / previous key),
/// or a random per-process key when it is not set (the token then only
/// verifies in this process).
fn signer() -> &'static Encryption {
    static SIGNER: OnceLock<Encryption> = OnceLock::new();
    SIGNER.get_or_init(|| {
        let keys = Keys::from_env(&ProcessEnv).unwrap_or_else(|error| {
            tracing::warn!(%error, "ENCRYPTION_KEY unusable; media tokens use a random process key");
            let mut bytes = [0u8; 32];
            ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes)
                .expect("system random");
            let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
            Keys::new(&hex).expect("random key")
        });
        Encryption::new(keys)
    })
}

fn default_avatar_url(urls: &TenantUrls) -> String {
    format!("{}{DEFAULT_AVATAR_PLACEHOLDER_PATH}", urls.app_url)
}

fn finalize(key: &str, urls: &TenantUrls) -> Option<String> {
    public_storage_url(key, &urls.storage_url).ok()
}

fn finalize_avatar_or_placeholder(avatar: &str, urls: &TenantUrls) -> Option<String> {
    if has_real_avatar(Some(avatar)) {
        finalize(avatar, urls)
    } else {
        Some(default_avatar_url(urls))
    }
}

/// `selectAvatarContactInbox`: the latest-messaged hydration-channel inbox,
/// else the first one.
fn select_inbox(inboxes: &[AvatarInbox]) -> Option<&AvatarInbox> {
    let mut hydration: Vec<&AvatarInbox> = inboxes
        .iter()
        .filter(|inbox| AVATAR_HYDRATION_CHANNELS.contains(&inbox.channel.as_str()))
        .collect();
    // `toSorted` is stable: ties keep their order.
    hydration.sort_by_key(|inbox| std::cmp::Reverse(inbox.last_message_at.unwrap_or(0)));
    hydration.first().copied().or_else(|| inboxes.first())
}

/// `resolveContactAvatarUrl` at `now_ms`.
pub fn resolve_contact_avatar_url(
    workspace_id: i64,
    avatar: Option<&str>,
    contact_inboxes: &[AvatarInbox],
    urls: &TenantUrls,
    now_ms: i64,
) -> Option<String> {
    let Some(inbox) = select_inbox(contact_inboxes) else {
        return finalize_avatar_or_placeholder(avatar.filter(|a| !a.is_empty())?, urls);
    };
    // `resolveAvatarUrl`.
    if let Some(avatar) = avatar.filter(|a| !a.is_empty()) {
        match parse_no_avatar_sentinel(avatar) {
            None => return finalize(avatar, urls),
            Some(failed_at) if is_no_avatar_sentinel_fresh(failed_at, now_ms) => {
                return Some(default_avatar_url(urls));
            }
            Some(_) => {}
        }
    }
    if !AVATAR_HYDRATION_CHANNELS.contains(&inbox.channel.as_str()) {
        return None;
    }
    let claims = MediaTokenClaims {
        workspace_id: workspace_id.to_string(),
        kind: MediaTokenKind::Avatar,
        ref_id: inbox.id.to_string(),
        message_created_at: None,
    };
    let token = signer()
        .sign_media_token(&claims, MEDIA_TOKEN_TTL_MS)
        .ok()?;
    Some(format!("{}/media/avatar/{token}", urls.app_url))
}
