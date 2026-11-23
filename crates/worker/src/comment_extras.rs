//! The `receiveComment` extras (Q.P7-V5): the Messenger comment lookup
//! (`getCommentAttachment`: live broadcast + photo / GIF), `resolveLiveComment`,
//! comment media (`downloadCommentMediaAttachment`,
//! `fetchThreadsCommentAttachments`), commenter avatars
//! (`downloadCommenterAvatar`) and post tracking (`channelPostService`).
//! Every step is best-effort except the post-tracking writes.

use std::sync::Arc;

use chatbotx_business::channel_api::{ChannelHttp, LiveChannelHttp};
use chatbotx_business::channel_post::{self, CommentPost};
use chatbotx_business::connection::credentials::{Upstream, UpstreamRequest};
use chatbotx_business::contact::channel_profile::{ProfileDeps, store_commenter_avatar};
use chatbotx_core::utils::image_optimize::ImageOptimizeProfile;
use chatbotx_core::utils::image_optimize_settings::EncodeSlots;
use chatbotx_core::utils::store_incoming_image::{
    StoreIncomingImageInput, store_incoming_image, to_stored_media_fields,
};
use chatbotx_files::{
    Acl, ChannelMediaLocation, FilesError, UploadFromUrlOptions, get_channel_media_prefix,
    guess_file_type_from_mime_type, upload_file_from_url,
};
use chatbotx_jobs::JobError;
use serde_json::Value;

use crate::channel::{ChannelCtx, InboundInbox, IncomingAttachment};
use crate::channels::whatsapp_media::FilesPutter;
use crate::channels::{messenger, threads};
use crate::{Deps, db_fail};

/// `COMMENT_MEDIA_MAX_BYTES`.
const COMMENT_MEDIA_MAX_BYTES: u64 = 25 * 1024 * 1024;
/// `LIVE_POST_TTL_SECONDS`: the 7-day private-reply window.
const LIVE_POST_TTL_SECONDS: u64 = 7 * 24 * 60 * 60;
/// `LIVE_VIDEO_STATUSES`.
const LIVE_VIDEO_STATUSES: &[&str] = &["LIVE", "LIVE_STOPPED"];

/// `CommentAttachmentLookup`.
pub(crate) struct MessengerLookup {
    pub is_live: bool,
    pub attachment: Option<IncomingAttachment>,
}

fn location(ctx: &ChannelCtx) -> ChannelMediaLocation {
    ChannelMediaLocation {
        channel: ctx.channel.clone(),
        workspace_id: ctx.workspace_id.to_string(),
        integration_id: ctx.integration_id.to_string(),
    }
}

async fn graph_get(
    upstream: &impl Upstream,
    url: &str,
    bearer: Option<&str>,
    query: &[(&str, &str)],
) -> Result<Value, String> {
    let mut url = url::Url::parse(url).map_err(|e| e.to_string())?;
    url.query_pairs_mut().extend_pairs(query.iter().copied());
    let headers = bearer
        .map(|t| vec![("Authorization".to_owned(), format!("Bearer {t}"))])
        .unwrap_or_default();
    let (status, body) = upstream
        .send(UpstreamRequest {
            method: "GET",
            url: url.to_string(),
            headers,
            body: None,
        })
        .await?;
    let parsed: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    if !(200..300).contains(&status) {
        return Err(parsed["error"]["message"]
            .as_str()
            .unwrap_or("Graph request failed")
            .to_owned());
    }
    Ok(parsed)
}

/// `getCommentAttachment`: `None` = the Graph lookup failed (`.catch`).
pub(crate) async fn messenger_comment_lookup(
    deps: &Deps,
    upstream: &impl Upstream,
    ctx: &ChannelCtx,
    comment_id: &str,
) -> Option<MessengerLookup> {
    let auth = messenger::auth_of(&ctx.auth).ok()?;
    let base = format!("{}/{}/{comment_id}", messenger::API_URL, auth.version);
    let token = auth.access_token.as_str();
    let fields = "attachment{type,url,unshimmed_url,target,media},live_broadcast_timestamp";
    let res = graph_get(upstream, &base, Some(token), &[("fields", fields)])
        .await
        .map_err(|error| tracing::warn!(comment_id, error, "receiveComment: comment lookup failed"))
        .ok()?;
    let timestamped = res["live_broadcast_timestamp"]
        .as_f64()
        .is_some_and(|t| t > 0.0);
    let is_live = timestamped && on_live_video(upstream, &base, token, comment_id).await;
    let attachment = &res["attachment"];
    let kind = attachment["type"].as_str().unwrap_or_default();
    let image = attachment
        .pointer("/media/image/src")
        .and_then(Value::as_str);
    let attachment = match (kind, image) {
        ("photo", Some(url)) => match store_photo(deps, ctx, url, Some(token), false).await {
            Ok(stored) => stored,
            Err(error) => {
                tracing::error!(error, "Failed to download comment attachment");
                None
            }
        },
        (kind, _) if kind.starts_with("animated_image") => {
            first_gif(deps, ctx, attachment, token).await
        }
        _ => None,
    };
    Some(MessengerLookup {
        is_live,
        attachment,
    })
}

/// `isOnLiveVideo`: a failed lookup or no named video trusts the timestamp.
async fn on_live_video(
    upstream: &impl Upstream,
    base: &str,
    token: &str,
    comment_id: &str,
) -> bool {
    match graph_get(
        upstream,
        base,
        Some(token),
        &[("fields", "object{live_status}")],
    )
    .await
    {
        Ok(res) if res["object"].is_object() => {
            LIVE_VIDEO_STATUSES.contains(&res["object"]["live_status"].as_str().unwrap_or_default())
        }
        Ok(_) => true,
        Err(error) => {
            tracing::warn!(
                comment_id,
                error,
                "Failed to confirm the comment's live video, trusting its broadcast timestamp"
            );
            true
        }
    }
}

fn https(link: Option<&str>) -> Option<String> {
    let url = url::Url::parse(link?).ok()?;
    (url.scheme() == "https").then(|| url.to_string())
}

/// `unwrapFacebookRedirectUrl`.
fn unwrap_redirect(link: Option<&str>) -> Option<String> {
    let url = url::Url::parse(link?).ok()?;
    let host = url.host_str()?;
    if !(host.ends_with("facebook.com") && url.path() == "/l.php") {
        return None;
    }
    let target = url.query_pairs().find(|(k, _)| k == "u")?.1.into_owned();
    https(Some(&target))
}

/// `gifAttachmentCandidates`: the original GIF (no token), then the video
/// rendition and the still preview (Facebook CDN, with the token).
fn gif_candidates(attachment: &Value) -> Vec<(String, bool)> {
    let original = https(attachment["unshimmed_url"].as_str())
        .or_else(|| unwrap_redirect(attachment["url"].as_str()))
        .or_else(|| unwrap_redirect(attachment.pointer("/target/url").and_then(Value::as_str)));
    let mut out = Vec::new();
    if let Some(url) = original {
        out.push((url, false));
    }
    for pointer in ["/media/source", "/media/image/src"] {
        if let Some(url) = attachment.pointer(pointer).and_then(Value::as_str) {
            out.push((url.to_owned(), true));
        }
    }
    out
}

async fn first_gif(
    deps: &Deps,
    ctx: &ChannelCtx,
    attachment: &Value,
    token: &str,
) -> Option<IncomingAttachment> {
    for (url, authorize) in gif_candidates(attachment) {
        match store_photo(deps, ctx, &url, authorize.then_some(token), true).await {
            Ok(Some(stored)) => return Some(stored),
            Ok(None) => {}
            Err(error) => tracing::warn!(url, error, "Failed to download comment GIF"),
        }
    }
    None
}

/// `getMessageAttachmentEntity` (`type: "image"`): the file re-hosted under
/// the channel media prefix (`media` profile). `require_media` rejects a
/// non image / video answer. `Ok(None)` = no file store.
async fn store_photo(
    deps: &Deps,
    ctx: &ChannelCtx,
    url: &str,
    bearer: Option<&str>,
    require_media: bool,
) -> Result<Option<IncomingAttachment>, String> {
    let Some(files) = deps.files.as_deref() else {
        return Ok(None);
    };
    let (bytes, mime) = LiveChannelHttp
        .download(url.to_owned(), bearer.map(str::to_owned))
        .await?
        .ok_or_else(|| format!("Failed to download attachment: {url}"))?;
    let mime = mime.unwrap_or_else(|| "image/png".to_owned());
    let file_type = guess_file_type_from_mime_type(&mime).as_str();
    if require_media && !matches!(file_type, "image" | "video") {
        return Err(format!(
            "Attachment is not an image or video ({mime}): {url}"
        ));
    }
    let key = format!(
        "{}/{}",
        get_channel_media_prefix(&location(ctx)),
        chatbotx_core::utils::id::create_id()
    );
    let stored = store_incoming_image(
        StoreIncomingImageInput {
            uploader: Some(&FilesPutter(files)),
            key: &key,
            bytes,
            mime_type: &mime,
            profile: ImageOptimizeProfile::Media,
            acl: Some("public-read"),
        },
        &chatbotx_core::env::ProcessEnv,
        &EncodeSlots::global(),
    )
    .await
    .map_err(|e| e.to_string())?;
    let fields = to_stored_media_fields(&stored);
    Ok(Some(IncomingAttachment {
        source_id: Some(chatbotx_core::utils::id::create_id()),
        file_type: file_type.to_owned(),
        mime_type: fields.mime_type,
        origin_path: fields.origin_path,
        size: Some(fields.size as i64),
        width: fields.width.map(i64::from),
        height: fields.height.map(i64::from),
        name: None,
        is_photo_message: true,
        optimized: stored.optimized,
    }))
}

/// `resolveLiveComment`: a post is remembered live for 7 days once any of
/// its comments is known live (a failed lookup mid-burst, a replay comment).
pub(crate) async fn resolve_live_comment(
    deps: &Deps,
    integration_id: i64,
    post_id: &str,
    lookup_is_live: Option<bool>,
) -> bool {
    let store = chatbotx_kv::DurableKv::new(deps.db.clone(), Arc::new(chatbotx_kv::SystemClock));
    let key = format!("comment-live-post:{integration_id}:{post_id}");
    if lookup_is_live == Some(true) {
        if let Err(error) = store.put(&key, &true, Some(LIVE_POST_TTL_SECONDS)).await {
            tracing::warn!(%error, integration_id, post_id, "resolveLiveComment: failed to remember live post");
        }
        return true;
    }
    match store.get::<bool>(&key).await {
        Ok(Some(true)) => return true,
        Ok(_) => {}
        Err(error) => {
            tracing::warn!(%error, integration_id, post_id, "resolveLiveComment: failed to read live post cache");
        }
    }
    if lookup_is_live.is_none() {
        tracing::warn!(
            integration_id,
            post_id,
            "resolveLiveComment: live status unknown, treating comment as a regular post comment"
        );
    }
    false
}

/// `downloadCommentMediaAttachment`: a third-party image / video re-hosted
/// without any channel token, behind the SSRF guard. Never fails.
pub(crate) async fn download_comment_media(
    deps: &Deps,
    ctx: &ChannelCtx,
    url: &str,
    comment_id: &str,
) -> Option<IncomingAttachment> {
    let files = deps.files.as_ref()?;
    let path = format!(
        "{}/{}",
        get_channel_media_prefix(&location(ctx)),
        chatbotx_core::utils::id::create_id()
    );
    let uploaded = async {
        let upload = files.upload_deps()?;
        let validate: chatbotx_files::ValidateUrl = Arc::new(|candidate: String| {
            Box::pin(async move {
                chatbotx_business::net::ssrf_guard::assert_public_url(
                    &candidate,
                    "Comment media URL",
                )
                .await
                .map_err(FilesError::UploadValidation)
            })
        });
        upload_file_from_url(
            &upload,
            url,
            &path,
            UploadFromUrlOptions {
                acl: Acl::PublicRead,
                max_bytes: Some(COMMENT_MEDIA_MAX_BYTES),
                validate_url: Some(validate),
                cancel: None,
            },
        )
        .await
    }
    .await;
    let uploaded = match uploaded {
        Ok(uploaded) => uploaded,
        Err(error) => {
            tracing::warn!(%error, comment_id, "downloadCommentMediaAttachment: failed to download comment media");
            return None;
        }
    };
    let file_type = uploaded.file_type.as_str();
    if !matches!(file_type, "image" | "video") {
        tracing::warn!(
            comment_id,
            mime_type = uploaded.mime_type,
            "downloadCommentMediaAttachment: comment media is not an image or video"
        );
        return None;
    }
    Some(IncomingAttachment {
        source_id: Some(chatbotx_core::utils::id::create_id()),
        file_type: file_type.to_owned(),
        mime_type: uploaded.mime_type,
        origin_path: uploaded.origin_path,
        size: Some(uploaded.size as i64),
        width: uploaded.width.map(i64::from),
        height: uploaded.height.map(i64::from),
        name: Some(uploaded.name),
        ..IncomingAttachment::default()
    })
}

/// `fetchThreadsCommentAttachments`: a GIF reply's `gif_url`, re-hosted.
pub(crate) async fn threads_comment_attachments(
    deps: &Deps,
    upstream: &impl Upstream,
    ctx: &ChannelCtx,
    comment_id: &str,
) -> Vec<IncomingAttachment> {
    let Ok(auth) = threads::auth_of(&ctx.auth) else {
        return Vec::new();
    };
    let url = format!("{}/{}/{comment_id}", threads::API_URL, auth.version);
    let query = [
        ("fields", "gif_url"),
        ("access_token", auth.access_token.as_str()),
    ];
    let gif = match graph_get(upstream, &url, None, &query).await {
        Ok(res) => res["gif_url"]
            .as_str()
            .filter(|u| !u.is_empty())
            .map(str::to_owned),
        Err(error) => {
            tracing::warn!(
                comment_id,
                error,
                "fetchThreadsCommentAttachments: failed to read reply gif_url"
            );
            None
        }
    };
    match gif {
        Some(gif) => download_comment_media(deps, ctx, &gif, comment_id)
            .await
            .into_iter()
            .collect(),
        None => Vec::new(),
    }
}

/// `downloadCommenterAvatar` + `setAvatarIfEmptyOrSentinel`, only when the
/// contact has no real avatar yet (a sentinel stays replaceable).
pub(crate) async fn commenter_avatar(
    deps: &Deps,
    workspace_id: i64,
    contact_id: i64,
    url: &str,
    bearer: Option<&str>,
    comment_id: &str,
) {
    let current = chatbotx_business::contact::channel_profile::contact_avatar(
        &deps.db,
        workspace_id,
        contact_id,
    )
    .await
    .ok()
    .flatten()
    .flatten();
    if chatbotx_core::utils::no_avatar_sentinel::has_real_avatar(current.as_deref()) {
        return;
    }
    let profile = ProfileDeps {
        db: &deps.db,
        http: &LiveChannelHttp,
        uploader: deps.files.as_deref(),
    };
    match store_commenter_avatar(profile, workspace_id, url, bearer).await {
        Ok(Some(avatar)) => {
            if let Err(error) =
                chatbotx_business::contact::channel_profile::set_avatar_if_empty_or_sentinel(
                    &deps.db,
                    workspace_id,
                    contact_id,
                    avatar,
                )
                .await
            {
                tracing::warn!(%error, comment_id, "receiveComment: failed to download commenter avatar");
            }
        }
        Ok(None) => {}
        Err(error) => {
            tracing::warn!(
                error,
                comment_id,
                "receiveComment: failed to download commenter avatar"
            );
        }
    }
}

/// `channelPostService.resolveForComment` + `contactInboxPostService.recordComment`
/// on a post-tracking channel: the `ChannelPost` id. A database error fails
/// the job (idempotent writes, before the message insert).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn track_post(
    deps: &Deps,
    upstream: &impl Upstream,
    inbox: &InboundInbox,
    ctx: &ChannelCtx,
    identifier: &str,
    contact_inbox_id: i64,
    post_id: &str,
    created_time: i64,
) -> Result<Option<i64>, JobError> {
    if !channel_post::supports_post_tracking(&inbox.channel) {
        return Ok(None);
    }
    let input = CommentPost {
        channel: inbox.channel.clone(),
        workspace_id: inbox.workspace_id,
        inbox_id: inbox.inbox_id,
        integration_id: ctx.integration_id,
        source_account_id: identifier.to_owned(),
        external_post_id: post_id.to_owned(),
    };
    let via_facebook = inbox.channel == "instagram" && {
        let id = ctx.integration_id;
        deps.db
            .read(move |conn| {
                Ok(conn
                    .query_row(
                        r#"SELECT "type" FROM "IntegrationInstagram" WHERE "id" = ?1"#,
                        [id],
                        |r| r.get::<_, Option<String>>(0),
                    )
                    .ok()
                    .flatten())
            })
            .await
            .map_err(db_fail)?
            .as_deref()
            == Some("facebook")
    };
    let fetch = || {
        channel_post::fetch_post_details(upstream, &inbox.channel, &ctx.auth, via_facebook, post_id)
    };
    let id = channel_post::resolve_for_comment(&deps.db, deps.files.as_ref(), &input, fetch)
        .await
        .map_err(|e| db_fail(e.to_string()))?;
    if let Some(id) = id {
        channel_post::record_comment(
            &deps.db,
            inbox.workspace_id,
            inbox.inbox_id,
            contact_inbox_id,
            id,
            created_time * 1000,
        )
        .await
        .map_err(|e| db_fail(e.to_string()))?;
    }
    Ok(id)
}

/// `live-comment.test.ts` (the Redis-failure case is n/a: the KV is the
/// database, and its errors are logged the same way).
#[cfg(test)]
mod tests {
    use super::*;
    use crate::contact_scan::tests::setup;

    async fn expires_in_ms(deps: &Deps, key: &'static str) -> i64 {
        deps.db
            .read(move |conn| {
                Ok(conn.query_row(
                    r#"SELECT "expiresAtMs" - CAST(unixepoch('subsec') * 1000 AS INTEGER) FROM "kv_entry" WHERE "key" = ?1"#,
                    [key],
                    |r| r.get::<_, i64>(0),
                )?)
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn a_live_lookup_remembers_the_post_for_seven_days() {
        let (deps, _) = setup().await;
        assert!(resolve_live_comment(&deps, 5, "post-1", Some(true)).await);
        let left = expires_in_ms(&deps, "comment-live-post:5:post-1").await;
        let week = (LIVE_POST_TTL_SECONDS * 1000) as i64;
        assert!(left > week - 60_000 && left <= week, "{left}");
    }

    #[tokio::test]
    async fn a_failed_lookup_falls_back_to_the_remembered_post() {
        let (deps, _) = setup().await;
        resolve_live_comment(&deps, 5, "post-1", Some(true)).await;
        assert!(resolve_live_comment(&deps, 5, "post-1", None).await);
        assert!(
            !resolve_live_comment(&deps, 6, "post-1", None).await,
            "the key is per integration"
        );
    }

    #[tokio::test]
    async fn a_known_live_post_stays_live_when_the_lookup_says_not_live() {
        let (deps, _) = setup().await;
        resolve_live_comment(&deps, 5, "post-1", Some(true)).await;
        assert!(resolve_live_comment(&deps, 5, "post-1", Some(false)).await);
    }

    #[tokio::test]
    async fn an_ordinary_comment_or_an_unknown_status_is_not_live() {
        let (deps, _) = setup().await;
        assert!(!resolve_live_comment(&deps, 5, "post-2", Some(false)).await);
        assert!(!resolve_live_comment(&deps, 5, "post-2", None).await);
    }
}
