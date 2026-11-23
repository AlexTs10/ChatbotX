//! Comment post tracking (Q.P7-V5): `channelPostService.resolveForComment`
//! (`packages/business/src/channel-post/service.ts`), the channel
//! `getPostDetails` handlers of Messenger / Instagram (`toChannelPostDetails`)
//! and `contactInboxPostService.recordComment`.

use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chatbotx_db::Db;
use chatbotx_db::repo::{new_id, now_ms};
use chatbotx_db::rusqlite::{self, Connection, OptionalExtension as _, params};
use chatbotx_files::{Acl, FilesError, UploadFromUrlOptions, Uploader, upload_file_from_url};
use serde_json::Value;

use crate::connection::credentials::{Upstream, UpstreamRequest};
use crate::errors::{BusinessError, Result};
use crate::tx;

/// `postTrackingChannels`.
pub const POST_TRACKING_CHANNELS: &[&str] = &["messenger", "instagram"];
pub const METADATA_FETCH_TIMEOUT: Duration = Duration::from_secs(15);
pub const METADATA_RETRY_INTERVAL_MS: i64 = 10 * 60 * 1000;
pub const METADATA_RETRY_MAX_AGE_MS: i64 = 24 * 60 * 60 * 1000;
pub const THUMBNAIL_MAX_BYTES: u64 = 30 * 1024 * 1024;

/// `supportsPostTracking`.
pub fn supports_post_tracking(channel: &str) -> bool {
    POST_TRACKING_CHANNELS.contains(&channel)
}

/// `ChannelPostDetails`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PostDetails {
    pub caption: Option<String>,
    pub media_type: Option<String>,
    pub permalink: Option<String>,
    /// Unix ms.
    pub published_at: Option<i64>,
    pub thumbnail_url: Option<String>,
}

/// `ResolveForCommentInput` (without `fetchDetails`).
#[derive(Debug, Clone)]
pub struct CommentPost {
    pub channel: String,
    pub workspace_id: i64,
    pub inbox_id: i64,
    pub integration_id: i64,
    pub source_account_id: String,
    pub external_post_id: String,
}

fn text(value: &Value) -> Option<String> {
    value.as_str().filter(|s| !s.is_empty()).map(str::to_owned)
}

/// A Graph time: RFC 3339 or Facebook's `+0000` offset.
fn graph_time(value: &Value) -> Option<i64> {
    let s = value.as_str()?;
    chrono::DateTime::parse_from_rfc3339(s)
        .or_else(|_| chrono::DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%z"))
        .ok()
        .map(|d| d.timestamp_millis())
}

/// `getPostDetails` of the Messenger / Instagram contact handlers, through
/// the inbox's integration (scoped by the workspace). `auth` = the row's
/// stored auth JSON; `instagram_via_facebook` = an Instagram row of type
/// `facebook`.
pub async fn fetch_post_details<U: Upstream>(
    upstream: &U,
    channel: &str,
    auth: &Value,
    instagram_via_facebook: bool,
    post_id: &str,
) -> std::result::Result<PostDetails, String> {
    if post_id.is_empty()
        || !post_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err("Invalid post id".into());
    }
    let token = auth["tokens"]["accessToken"].as_str().unwrap_or_default();
    let (base, fields) = match channel {
        "messenger" => {
            let version = auth["version"]
                .as_str()
                .filter(|v| !v.is_empty())
                .unwrap_or("v23.0");
            (
                format!("https://graph.facebook.com/{version}/{post_id}"),
                "message,full_picture,created_time,permalink_url",
            )
        }
        _ => {
            let (host, default) = if instagram_via_facebook {
                ("https://graph.facebook.com", "v23.0")
            } else {
                ("https://graph.instagram.com", "v22.0")
            };
            let version = auth["metadata"]["version"].as_str().unwrap_or(default);
            (
                format!("{host}/{version}/{post_id}"),
                "caption,media_type,media_url,thumbnail_url,timestamp,permalink",
            )
        }
    };
    let mut url = url::Url::parse(&base).map_err(|e| e.to_string())?;
    url.query_pairs_mut().append_pair("fields", fields);
    let (status, body) = upstream
        .send(UpstreamRequest {
            method: "GET",
            url: url.to_string(),
            headers: vec![("Authorization".into(), format!("Bearer {token}"))],
            body: None,
        })
        .await?;
    let raw: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    if !(200..300).contains(&status) {
        return Err(raw["error"]["message"]
            .as_str()
            .unwrap_or("Post details request failed")
            .to_owned());
    }
    Ok(match channel {
        "messenger" => PostDetails {
            caption: text(&raw["message"]),
            media_type: None,
            permalink: text(&raw["permalink_url"]),
            published_at: graph_time(&raw["created_time"]),
            thumbnail_url: text(&raw["full_picture"]),
        },
        _ => PostDetails {
            caption: text(&raw["caption"]),
            media_type: text(&raw["media_type"]),
            permalink: text(&raw["permalink"]),
            published_at: graph_time(&raw["timestamp"]),
            thumbnail_url: text(&raw["thumbnail_url"]).or_else(|| text(&raw["media_url"])),
        },
    })
}

fn is_foreign_key(error: &BusinessError) -> bool {
    error.to_string().contains("FOREIGN KEY constraint failed")
}

/// `resolveForComment`: the `ChannelPost` id of the commented post (`None` =
/// the workspace was deleted mid-flight). The insert winner fetches the
/// metadata once (15 s budget, best-effort); a post still without metadata
/// is retried at most every 10 minutes for its first day. A database error
/// propagates (the job retries; the writes are idempotent).
pub async fn resolve_for_comment<F, Fut>(
    db: &Db,
    files: Option<&Arc<Uploader>>,
    input: &CommentPost,
    fetch_details: F,
) -> Result<Option<i64>>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = std::result::Result<PostDetails, String>>,
{
    match resolve(db, files, input, fetch_details).await {
        Err(error) if is_foreign_key(&error) => {
            tracing::info!(
                external_post_id = input.external_post_id,
                workspace_id = input.workspace_id,
                "channel-post: workspace was deleted before post could be recorded"
            );
            Ok(None)
        }
        other => other.map(Some),
    }
}

/// `(id, integrationId, metadataFetchedAt)`.
type Existing = (i64, i64, Option<i64>);

fn find(conn: &Connection, input: &CommentPost) -> rusqlite::Result<Option<Existing>> {
    conn.query_row(
        r#"SELECT "id", "integrationId", "metadataFetchedAt" FROM "ChannelPost"
           WHERE "workspaceId" = ?1 AND "channel" = ?2 AND "externalPostId" = ?3 LIMIT 1"#,
        params![input.workspace_id, input.channel, input.external_post_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .optional()
}

fn insert_bare(conn: &Connection, input: &CommentPost) -> rusqlite::Result<Option<i64>> {
    let now = now_ms();
    conn.query_row(
        r#"INSERT INTO "ChannelPost" ("id", "createdAt", "updatedAt", "workspaceId", "inboxId", "channel",
               "integrationId", "sourceAccountId", "externalPostId", "metadataAttemptedAt")
           VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?2)
           ON CONFLICT ("workspaceId", "channel", "externalPostId") DO NOTHING
           RETURNING "id""#,
        params![
            new_id(),
            now,
            input.workspace_id,
            input.inbox_id,
            input.channel,
            input.integration_id,
            input.source_account_id,
            input.external_post_id
        ],
        |r| r.get(0),
    )
    .optional()
}

enum Claim {
    Existing(Existing),
    Inserted(i64),
}

async fn resolve<F, Fut>(
    db: &Db,
    files: Option<&Arc<Uploader>>,
    input: &CommentPost,
    fetch_details: F,
) -> Result<i64>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = std::result::Result<PostDetails, String>>,
{
    let key = input.clone();
    // One write transaction: find, else claim the row with a bare insert.
    let claim = tx::write(db, move |conn| {
        if let Some(existing) = find(conn, &key)? {
            return Ok(Claim::Existing(existing));
        }
        match insert_bare(conn, &key)? {
            Some(id) => Ok(Claim::Inserted(id)),
            None => find(conn, &key)?.map(Claim::Existing).ok_or_else(|| {
                BusinessError::exception(
                    "Channel post was not available after insert race",
                    "systemError",
                    500,
                )
            }),
        }
    })
    .await?;
    let (id, fetch) = match claim {
        Claim::Inserted(id) => (id, true),
        Claim::Existing((id, integration_id, fetched_at)) => {
            let (key, ws) = (input.clone(), input.workspace_id);
            let claimed = tx::write(db, move |conn| {
                if integration_id != key.integration_id {
                    conn.execute(
                        r#"UPDATE "ChannelPost" SET "integrationId" = ?4, "inboxId" = ?5, "sourceAccountId" = ?6, "updatedAt" = ?7
                           WHERE "workspaceId" = ?1 AND "channel" = ?2 AND "externalPostId" = ?3 AND "integrationId" IS NOT ?4"#,
                        params![ws, key.channel, key.external_post_id, key.integration_id, key.inbox_id, key.source_account_id, now_ms()],
                    )?;
                }
                if fetched_at.is_some() {
                    return Ok(false);
                }
                let now = now_ms();
                Ok(conn.execute(
                    r#"UPDATE "ChannelPost" SET "metadataAttemptedAt" = ?3, "updatedAt" = ?3
                       WHERE "id" = ?1 AND "workspaceId" = ?2 AND "metadataFetchedAt" IS NULL
                         AND "createdAt" > ?3 - ?4 AND "metadataAttemptedAt" < ?3 - ?5"#,
                    params![id, ws, now, METADATA_RETRY_MAX_AGE_MS, METADATA_RETRY_INTERVAL_MS],
                )? > 0)
            })
            .await?;
            (id, claimed)
        }
    };
    if fetch {
        fetch_and_save_metadata(db, files, input.workspace_id, id, fetch_details).await;
    }
    Ok(id)
}

/// `fetchAndSaveMetadata`: never fails (a failure marks the attempt).
async fn fetch_and_save_metadata<F, Fut>(
    db: &Db,
    files: Option<&Arc<Uploader>>,
    workspace_id: i64,
    id: i64,
    fetch_details: F,
) where
    F: FnOnce() -> Fut,
    Fut: Future<Output = std::result::Result<PostDetails, String>>,
{
    let deadline = Instant::now() + METADATA_FETCH_TIMEOUT;
    let details = match tokio::time::timeout(METADATA_FETCH_TIMEOUT, fetch_details()).await {
        Ok(Ok(details)) => details,
        Ok(Err(error)) => return metadata_failed(db, workspace_id, id, &error).await,
        Err(_) => {
            return metadata_failed(
                db,
                workspace_id,
                id,
                "Channel post metadata fetch timed out",
            )
            .await;
        }
    };
    let remaining = deadline.saturating_duration_since(Instant::now());
    let thumbnail = match (details.thumbnail_url.as_deref(), files) {
        (Some(url), Some(files)) if !remaining.is_zero() => {
            rehost_thumbnail(files, workspace_id, id, url, remaining).await
        }
        _ => None,
    };
    let saved = tx::write(db, move |conn| {
        conn.execute(
            r#"UPDATE "ChannelPost" SET "caption" = ?3, "mediaType" = ?4, "permalink" = ?5, "publishedAt" = ?6,
                   "thumbnail" = ?7, "metadataFetchedAt" = ?8, "updatedAt" = ?8
               WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            params![
                id,
                workspace_id,
                details.caption,
                details.media_type,
                details.permalink,
                details.published_at,
                thumbnail,
                now_ms()
            ],
        )?;
        Ok(())
    })
    .await;
    if let Err(error) = saved {
        metadata_failed(db, workspace_id, id, &error.to_string()).await;
    }
}

async fn metadata_failed(db: &Db, workspace_id: i64, id: i64, error: &str) {
    let marked = tx::write(db, move |conn| {
        conn.execute(
            r#"UPDATE "ChannelPost" SET "metadataAttemptedAt" = ?3 WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            params![id, workspace_id, now_ms()],
        )?;
        Ok(())
    })
    .await;
    if let Err(mark) = marked {
        tracing::warn!(%mark, channel_post_id = id, workspace_id, "channel-post: failed to mark metadata attempt");
    }
    tracing::warn!(
        error,
        channel_post_id = id,
        workspace_id,
        "channel-post: failed to fetch post metadata"
    );
}

/// `rehostThumbnail`: best-effort, within the remaining budget, SSRF-checked.
async fn rehost_thumbnail(
    files: &Arc<Uploader>,
    workspace_id: i64,
    id: i64,
    url: &str,
    budget: Duration,
) -> Option<String> {
    let path = format!(
        "public/space/{workspace_id}/channel-posts/{id}/{}",
        chatbotx_core::utils::id::create_id()
    );
    let upload = async {
        let deps = files.upload_deps()?;
        let validate: chatbotx_files::ValidateUrl = Arc::new(|candidate: String| {
            Box::pin(async move {
                crate::net::ssrf_guard::assert_public_url(&candidate, "Post thumbnail URL")
                    .await
                    .map_err(FilesError::UploadValidation)
            })
        });
        upload_file_from_url(
            &deps,
            url,
            &path,
            UploadFromUrlOptions {
                acl: Acl::PublicRead,
                max_bytes: Some(THUMBNAIL_MAX_BYTES),
                validate_url: Some(validate),
                cancel: None,
            },
        )
        .await
    };
    match tokio::time::timeout(budget, upload).await {
        Ok(Ok(uploaded)) => Some(uploaded.origin_path),
        Ok(Err(error)) => {
            tracing::warn!(%error, channel_post_id = id, workspace_id, "channel-post: failed to re-host post thumbnail");
            None
        }
        Err(_) => {
            tracing::warn!(
                channel_post_id = id,
                workspace_id,
                "channel-post: failed to re-host post thumbnail (timeout)"
            );
            None
        }
    }
}

/// `contactInboxPostService.recordComment`: links the contact inbox to the
/// post only when the post is on the contact inbox's own channel and the
/// inbox is the workspace's.
pub async fn record_comment(
    db: &Db,
    workspace_id: i64,
    inbox_id: i64,
    contact_inbox_id: i64,
    post_id: i64,
    commented_at: i64,
) -> Result<bool> {
    tx::write(db, move |conn| {
        Ok(conn.execute(
            r#"INSERT INTO "ContactInboxPost" ("workspaceId", "contactInboxId", "postId", "commentedAt")
               SELECT ?1, ci."id", ?4, ?5 FROM "ContactInbox" ci
               JOIN "Inbox" i ON i."id" = ci."inboxId" AND i."workspaceId" = ?1
               WHERE ci."id" = ?3 AND ci."inboxId" = ?2
                 AND EXISTS (SELECT 1 FROM "ChannelPost" cp WHERE cp."id" = ?4 AND cp."workspaceId" = ?1 AND cp."channel" = ci."channel")
               ON CONFLICT ("workspaceId", "contactInboxId", "postId") DO NOTHING"#,
            params![workspace_id, inbox_id, contact_inbox_id, post_id, commented_at],
        )? > 0)
    })
    .await
}
