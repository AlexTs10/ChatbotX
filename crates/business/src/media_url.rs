//! The attachment half of `media/resolve-media-url.ts` and
//! `message/list-for-conversation.ts` `presignAttachments` /
//! `withResolvedAttachmentUrls`: a stored object gets a presigned storage URL,
//! a pending (lazily ingested) one on a hydration channel the signed
//! `/media/attachment/<token>` proxy URL, a failed one no URL. The avatar half
//! lives in `contact::avatar`.
//!
//! ponytail: the proxy URL uses the platform app URL (`resolveWorkspaceAppUrl`
//! white-label domains are tenant work, E16).

use chatbotx_core::encryption::{MEDIA_TOKEN_TTL_MS, MediaTokenClaims, MediaTokenKind};
use chatbotx_db::Db;
use chatbotx_db::repo::message::MessageWithAttachments;
use chatbotx_files::Uploader;
use serde_json::Value;

use crate::tx;

/// `HYDRATION_CHANNELS`.
pub const HYDRATION_CHANNELS: &[&str] = &["messenger", "instagram", "whatsapp"];
/// `FAILED_PREFIX`.
pub const FAILED_PREFIX: &str = "failed:";
/// `WA_MEDIA_PREFIX`.
pub const WA_MEDIA_PREFIX: &str = "wa-media:";
/// `FALLBACK_TOKEN_TTL_MS`: outlives the 1 h presigned URL it backs up.
pub const FALLBACK_TOKEN_TTL_MS: i64 = 24 * 60 * 60 * 1000;

/// `isPendingOriginPath`: a provider URL or a `wa-media:` id.
pub fn is_pending_origin_path(origin_path: &str) -> bool {
    origin_path.starts_with("http://")
        || origin_path.starts_with("https://")
        || origin_path.starts_with(WA_MEDIA_PREFIX)
}

/// `isFailedOriginPath`.
pub fn is_failed_origin_path(origin_path: &str) -> bool {
    origin_path.starts_with(FAILED_PREFIX)
}

/// `HTTP_URL_RE` (`/^https?:\/\//i`).
fn is_http_url(key: &str) -> bool {
    let lower = key.get(..8).unwrap_or(key).to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

/// One attachment to resolve (`AttachmentMediaRef` without `kind`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentRef<'a> {
    pub workspace_id: i64,
    pub attachment_id: i64,
    pub origin_path: &'a str,
    pub channel: &'a str,
    /// The parent message `createdAt` (signed in as the shard hint).
    pub message_created_at: Option<i64>,
}

/// `buildProxyUrl` (`kind: "attachment"`): `<appUrl>/media/attachment/<token>`.
pub fn attachment_proxy_url(app_url: &str, r: &AttachmentRef<'_>, ttl_ms: i64) -> Option<String> {
    let claims = MediaTokenClaims {
        workspace_id: r.workspace_id.to_string(),
        kind: MediaTokenKind::Attachment,
        ref_id: r.attachment_id.to_string(),
        message_created_at: r.message_created_at,
    };
    let token = sign_media_token(&claims, ttl_ms)?;
    Some(format!(
        "{}/media/attachment/{token}",
        app_url.trim_end_matches('/')
    ))
}

/// `signMediaToken` with the process signer (`None` = it could not sign).
pub fn sign_media_token(claims: &MediaTokenClaims, ttl_ms: i64) -> Option<String> {
    crate::contact::avatar::signer()
        .sign_media_token(claims, ttl_ms)
        .ok()
}

/// `verifyMediaToken` with the process signer (`None` = invalid or expired).
pub fn verify_media_token(token: &str) -> Option<MediaTokenClaims> {
    crate::contact::avatar::signer()
        .verify_media_token(token)
        .ok()
        .map(|payload| payload.claims)
}

/// `resolveMediaUrl({ kind: "attachment" }, finalize)`: `finalize` maps a
/// stored key (`None` = it could not be resolved).
pub async fn resolve_attachment_url<F, Fut>(
    r: &AttachmentRef<'_>,
    app_url: &str,
    finalize: F,
) -> Option<String>
where
    F: FnOnce(String) -> Fut,
    Fut: std::future::Future<Output = Option<String>>,
{
    if is_failed_origin_path(r.origin_path) {
        return None;
    }
    if HYDRATION_CHANNELS.contains(&r.channel) && is_pending_origin_path(r.origin_path) {
        return attachment_proxy_url(app_url, r, MEDIA_TOKEN_TTL_MS);
    }
    finalize(r.origin_path.to_owned()).await
}

/// `resolveAttachmentFallbackUrl`: the proxy URL a client tries when the
/// first URL fails; only hydration channels have that recovery path.
pub fn resolve_attachment_fallback_url(r: &AttachmentRef<'_>, app_url: &str) -> Option<String> {
    if !HYDRATION_CHANNELS.contains(&r.channel) || is_failed_origin_path(r.origin_path) {
        return None;
    }
    attachment_proxy_url(app_url, r, FALLBACK_TOKEN_TTL_MS)
}

/// Where attachment URLs come from: the storage presigner and the app URL.
#[derive(Clone, Copy)]
pub struct MediaUrls<'a> {
    pub uploader: Option<&'a Uploader>,
    pub app_url: &'a str,
}

/// The `uploader.getPresignedDownload` finalizer (an http URL stays as it is).
async fn presign(uploader: Option<&Uploader>, key: String) -> Option<String> {
    if is_http_url(&key) {
        return Some(key);
    }
    match uploader?.get_presigned_download(&key, None).await {
        Ok(url) => Some(url),
        Err(error) => {
            // One unresolvable attachment must not fail the whole page.
            tracing::warn!(%error, "Failed to resolve attachment media URL; omitting");
            None
        }
    }
}

/// `withResolvedAttachmentUrls`: [`crate::message::message_json`] with each
/// attachment's `url` / `fallbackUrl`.
pub async fn message_json_with_resolved_urls(
    message: &MessageWithAttachments,
    channel: &str,
    urls: MediaUrls<'_>,
) -> Value {
    let mut out = crate::message::message_json(message);
    if let Some(Value::Array(items)) = out.get_mut("attachments") {
        for (item, attachment) in items.iter_mut().zip(&message.attachments) {
            let r = AttachmentRef {
                workspace_id: attachment.workspace_id,
                attachment_id: attachment.id,
                origin_path: &attachment.origin_path,
                channel,
                message_created_at: Some(message.message.created_at),
            };
            let url =
                resolve_attachment_url(&r, urls.app_url, |key| presign(urls.uploader, key)).await;
            item["url"] = url.map_or(Value::Null, Value::String);
            item["fallbackUrl"] = resolve_attachment_fallback_url(&r, urls.app_url)
                .map_or(Value::Null, Value::String);
        }
    }
    out
}

/// `loadContactInboxChannels` + [`message_json_with_resolved_urls`] for a page.
pub async fn messages_json_with_resolved_urls(
    db: &Db,
    workspace_id: i64,
    messages: &[MessageWithAttachments],
    urls: MediaUrls<'_>,
) -> crate::Result<Vec<Value>> {
    let mut ids: Vec<i64> = messages
        .iter()
        .map(|m| m.message.contact_inbox_id)
        .collect();
    ids.sort_unstable();
    ids.dedup();
    let channels: Vec<(i64, String)> = if ids.is_empty() {
        Vec::new()
    } else {
        let list = chatbotx_db::repo::id_list(&ids);
        tx::read(db, move |conn| {
            let mut stmt = conn
                .prepare(
                    r#"SELECT ci."id", ci."channel" FROM "ContactInbox" ci
                       JOIN "Inbox" i ON i."id" = ci."inboxId"
                       WHERE i."workspaceId" = ?1 AND ci."id" IN (SELECT value FROM json_each(?2))"#,
                )
                .map_err(|e| crate::BusinessError::Repo(e.into()))?;
            let rows = stmt
                .query_map(chatbotx_db::rusqlite::params![workspace_id, list], |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })
                .and_then(|rows| rows.collect::<chatbotx_db::rusqlite::Result<Vec<_>>>())
                .map_err(|e| crate::BusinessError::Repo(e.into()))?;
            Ok(rows)
        })
        .await?
    };
    let mut out = Vec::with_capacity(messages.len());
    for message in messages {
        let channel = channels
            .iter()
            .find(|(id, _)| *id == message.message.contact_inbox_id)
            .map_or("", |(_, channel)| channel.as_str());
        out.push(message_json_with_resolved_urls(message, channel, urls).await);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_path_kinds() {
        assert!(is_pending_origin_path("https://lookaside.fbsbx.com/x"));
        assert!(is_pending_origin_path("wa-media:1"));
        assert!(!is_pending_origin_path("public/space/1/a.png"));
        assert!(is_failed_origin_path("failed:unresolvable"));
        assert!(is_http_url("HTTPS://x"));
        assert!(!is_http_url("public/x"));
    }
}
