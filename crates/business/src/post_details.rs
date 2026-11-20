//! `features/conversations/queries/get-post-details.query.ts`
//! (`getPostDetailsQuery`): the post a comment conversation sits on, read
//! from its channel (Messenger, Instagram Login / via Facebook, Threads,
//! TikTok `business/videos/list/`). Every integration lookup is scoped by
//! the inbox AND the workspace.
//!
//! ponytail: the 24 h / 5 min cache is in-process, and a TikTok re-authorize
//! does not drop its entries (the original's cache tag); they expire.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use chatbotx_db::Db;
use chatbotx_db::rusqlite::OptionalExtension as _;
use serde_json::{Map, Value, json};

use crate::connection::credentials::{Upstream, UpstreamRequest};
use crate::errors::{BusinessError, Result};
use crate::tx;

/// `POST_DETAILS_CHANNELS`.
pub const POST_DETAILS_CHANNELS: &[&str] = &["messenger", "instagram", "threads", "tiktok"];
/// `POST_DETAILS_CACHE_TTL`.
pub const CACHE_TTL: Duration = Duration::from_secs(60 * 60 * 24);
/// `DEGRADED_POST_DETAILS_CACHE_TTL`.
pub const DEGRADED_CACHE_TTL: Duration = Duration::from_secs(60 * 5);
const TIKTOK_BUSINESS_API_URL: &str = "https://business-api.tiktok.com/open_api/v1.3/";

static CACHE: LazyLock<Mutex<HashMap<String, (Instant, Duration, Value)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// `getPostDetailsCacheKey`: the workspace is part of the key.
pub fn cache_key(workspace_id: i64, inbox_id: &str, post_id: &str) -> String {
    format!("post-details:{workspace_id}:{inbox_id}:{post_id}")
}

/// The integration row of an inbox: `(external id, name, auth, type)`.
type Row = (String, String, Value, Option<String>);

/// `<service>.findByInboxIdForWorkspace` (`findOrFail`: "Record not found").
async fn integration(db: &Db, sql: &'static str, workspace_id: i64, inbox_id: &str) -> Result<Row> {
    // A non-numeric inbox id names no row.
    let Ok(inbox_id) = inbox_id.parse::<i64>() else {
        return Err(BusinessError::not_found("Record not found"));
    };
    tx::read(db, move |conn| {
        conn.query_row(
            sql,
            chatbotx_db::rusqlite::params![workspace_id, inbox_id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    serde_json::from_str(&r.get::<_, String>(2)?).unwrap_or(Value::Null),
                    r.get::<_, Option<String>>(3)?,
                ))
            },
        )
        .optional()
        .map_err(|e| BusinessError::Repo(e.into()))?
        .ok_or_else(|| BusinessError::not_found("Record not found"))
    })
    .await
}

const MESSENGER_SQL: &str = r#"SELECT "pageId", "name", "auth", NULL FROM "IntegrationMessenger"
    WHERE "workspaceId" = ?1 AND "inboxId" = ?2 LIMIT 1"#;
const INSTAGRAM_SQL: &str = r#"SELECT "igId", "name", "auth", "type" FROM "IntegrationInstagram"
    WHERE "workspaceId" = ?1 AND "inboxId" = ?2 LIMIT 1"#;
const THREADS_SQL: &str = r#"SELECT "threadsUserId", "username", "auth", NULL FROM "IntegrationThreads"
    WHERE "workspaceId" = ?1 AND "inboxId" = ?2 LIMIT 1"#;
const TIKTOK_SQL: &str = r#"SELECT "openId", "name", "auth", NULL FROM "IntegrationTiktok"
    WHERE "workspaceId" = ?1 AND "inboxId" = ?2 LIMIT 1"#;

fn token(auth: &Value) -> String {
    auth["tokens"]["accessToken"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

/// A Graph id goes into a URL path: only id characters (no `/`, `?`, `..`).
fn valid_id(post_id: &str) -> bool {
    !post_id.is_empty()
        && post_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// One Graph `GET` (`rescue`: a non-2xx answer is the provider's message).
async fn graph_get<U: Upstream>(
    upstream: &U,
    url: url::Url,
    bearer: Option<&str>,
) -> Result<Value> {
    let headers = bearer
        .map(|t| vec![("Authorization".into(), format!("Bearer {t}"))])
        .unwrap_or_default();
    let (status, body) = upstream
        .send(UpstreamRequest {
            method: "GET",
            url: url.to_string(),
            headers,
            body: None,
        })
        .await
        .map_err(|e| BusinessError::exception(e, "systemError", 400))?;
    let parsed: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    if !(200..300).contains(&status) {
        let message = parsed["error"]["message"]
            .as_str()
            .unwrap_or("Post details request failed");
        return Err(BusinessError::exception(message, "systemError", 400));
    }
    Ok(parsed)
}

fn url(base: &str, pairs: &[(&str, &str)]) -> url::Url {
    let mut url = url::Url::parse(base).expect("fixed host");
    url.query_pairs_mut().extend_pairs(pairs);
    url
}

/// `resolvePostPreviewImage`: a video shows its thumbnail only.
fn preview_image(raw: &Value) -> Value {
    let video = raw["media_type"]
        .as_str()
        .is_some_and(|t| t.eq_ignore_ascii_case("VIDEO"));
    let thumb = raw.get("thumbnail_url").filter(|v| !v.is_null());
    if video {
        return thumb.cloned().unwrap_or(Value::Null);
    }
    raw.get("media_url")
        .filter(|v| !v.is_null())
        .or(thumb)
        .cloned()
        .unwrap_or(Value::Null)
}

/// `PostDetails` without the keys that are `undefined`.
fn details(fields: Vec<(&str, Value)>) -> Value {
    let map: Map<String, Value> = fields
        .into_iter()
        .filter(|(_, v)| !v.is_null())
        .map(|(k, v)| (k.to_owned(), v))
        .collect();
    Value::Object(map)
}

/// Resolves the post on the channel: `(details, degraded)`.
async fn fetch<U: Upstream>(
    db: &Db,
    upstream: &U,
    workspace_id: i64,
    inbox_id: &str,
    post_id: &str,
    channel: &str,
) -> Result<(Value, bool)> {
    match channel {
        "instagram" => {
            let (ig_id, name, auth, kind) =
                integration(db, INSTAGRAM_SQL, workspace_id, inbox_id).await?;
            let via_facebook = kind.as_deref() == Some("facebook");
            let (host, default_version) = if via_facebook {
                ("https://graph.facebook.com", "v23.0")
            } else {
                ("https://graph.instagram.com", "v22.0")
            };
            let version = auth["metadata"]["version"]
                .as_str()
                .unwrap_or(default_version);
            let raw = graph_get(
                upstream,
                url(
                    &format!("{host}/{version}/{post_id}"),
                    &[(
                        "fields",
                        "caption,media_type,media_url,thumbnail_url,timestamp,permalink",
                    )],
                ),
                Some(&token(&auth)),
            )
            .await?;
            let picture = raw
                .get("thumbnail_url")
                .filter(|v| !v.is_null())
                .or_else(|| raw.get("media_url"))
                .cloned()
                .unwrap_or(Value::Null);
            Ok((
                details(vec![
                    ("text", raw["caption"].clone()),
                    ("picture", picture),
                    ("from", json!({ "id": ig_id, "name": name })),
                    ("createdAt", raw["timestamp"].clone()),
                    ("link", raw["permalink"].clone()),
                ]),
                false,
            ))
        }
        "threads" => {
            let (_, _, auth, _) = integration(db, THREADS_SQL, workspace_id, inbox_id).await?;
            let version = auth["metadata"]["version"].as_str().unwrap_or("v1.0");
            let access = token(&auth);
            let raw = graph_get(
                upstream,
                url(
                    &format!("https://graph.threads.com/{version}/{post_id}"),
                    &[
                        (
                            "fields",
                            "text,permalink,media_type,media_url,thumbnail_url,timestamp,username,owner",
                        ),
                        ("access_token", &access),
                    ],
                ),
                None,
            )
            .await?;
            let from = raw["username"].as_str().map_or(Value::Null, |username| {
                json!({ "id": raw["owner"]["id"].as_str().unwrap_or(username), "name": username })
            });
            let created_at = match &raw["timestamp"] {
                Value::Null => json!(chatbotx_db::repo::iso(chatbotx_db::repo::now_ms())),
                other => other.clone(),
            };
            Ok((
                details(vec![
                    ("text", raw["text"].clone()),
                    ("picture", preview_image(&raw)),
                    ("from", from),
                    ("createdAt", created_at),
                    ("link", raw["permalink"].clone()),
                ]),
                false,
            ))
        }
        "tiktok" => tiktok(db, upstream, workspace_id, inbox_id, post_id).await,
        _ => {
            let (_, _, auth, _) = integration(db, MESSENGER_SQL, workspace_id, inbox_id).await?;
            let version = auth["version"].as_str().unwrap_or("v23.0");
            let raw = graph_get(
                upstream,
                url(
                    &format!("https://graph.facebook.com/{version}/{post_id}"),
                    &[(
                        "fields",
                        "message,full_picture,from,created_time,permalink_url",
                    )],
                ),
                Some(&token(&auth)),
            )
            .await?;
            Ok((
                details(vec![
                    ("text", raw["message"].clone()),
                    ("picture", raw["full_picture"].clone()),
                    ("from", raw["from"].clone()),
                    ("createdAt", raw["created_time"].clone()),
                ]),
                false,
            ))
        }
    }
}

/// `tiktokIntegrationService.getPostDetails`: the derived link without the
/// `video.list` scope; the video's caption / thumbnail / time with it; a
/// failed or empty lookup is `degraded` (the short TTL).
async fn tiktok<U: Upstream>(
    db: &Db,
    upstream: &U,
    workspace_id: i64,
    inbox_id: &str,
    post_id: &str,
) -> Result<(Value, bool)> {
    let (open_id, name, auth, _) = integration(db, TIKTOK_SQL, workspace_id, inbox_id).await?;
    let from = json!({ "id": open_id, "name": name });
    // `buildTiktokVideoUrl`.
    let link = auth["metadata"]["username"]
        .as_str()
        .filter(|u| !u.is_empty())
        .map_or(Value::Null, |u| {
            json!(format!("https://www.tiktok.com/@{u}/video/{post_id}"))
        });
    let fallback = || details(vec![("from", from.clone()), ("link", link.clone())]);
    let can_list = auth["metadata"]["scopes"]
        .as_array()
        .is_some_and(|s| s.iter().any(|s| s == "video.list"));
    if !can_list {
        return Ok((fallback(), false));
    }
    let business_id = auth["metadata"]["openId"].as_str().unwrap_or_default();
    let fields = json!([
        "item_id",
        "caption",
        "thumbnail_url",
        "share_url",
        "create_time",
        "comments"
    ])
    .to_string();
    let filters = json!({ "video_ids": [post_id] }).to_string();
    let request = url(
        &format!("{TIKTOK_BUSINESS_API_URL}business/videos/list/"),
        &[
            ("business_id", business_id),
            ("fields", &fields),
            ("max_count", "1"),
            ("filters", &filters),
        ],
    );
    let reply = upstream
        .send(UpstreamRequest {
            method: "GET",
            url: request.to_string(),
            headers: vec![("Access-Token".into(), token(&auth))],
            body: None,
        })
        .await;
    let video = match reply {
        Ok((status, body)) if (200..300).contains(&status) => {
            let parsed: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
            if parsed["code"].as_i64() != Some(0) {
                tracing::warn!(
                    inbox_id,
                    post_id,
                    "Unable to load TikTok video details; falling back to the derived link"
                );
                return Ok((fallback(), true));
            }
            parsed["data"]["videos"]
                .as_array()
                .and_then(|v| v.iter().find(|v| v["item_id"] == json!(post_id)).cloned())
        }
        _ => {
            tracing::warn!(
                inbox_id,
                post_id,
                "Unable to load TikTok video details; falling back to the derived link"
            );
            return Ok((fallback(), true));
        }
    };
    let Some(video) = video else {
        return Ok((fallback(), true));
    };
    let created_at = video["create_time"]
        .as_i64()
        .filter(|t| *t != 0)
        .map_or(Value::Null, |t| json!(chatbotx_db::repo::iso(t * 1000)));
    let link = match &video["share_url"] {
        Value::String(url) => json!(url),
        _ => link,
    };
    Ok((
        details(vec![
            ("text", video["caption"].clone()),
            ("picture", video["thumbnail_url"].clone()),
            ("from", from),
            ("createdAt", created_at),
            ("link", link),
        ]),
        false,
    ))
}

/// `getPostDetailsQuery` (cached; a degraded TikTok answer for 5 minutes).
pub async fn get_post_details<U: Upstream>(
    db: &Db,
    upstream: &U,
    workspace_id: i64,
    inbox_id: &str,
    post_id: &str,
    channel: &str,
) -> Result<Value> {
    if !POST_DETAILS_CHANNELS.contains(&channel) {
        return Err(BusinessError::exception(
            format!("Post details are not available for the {channel} channel"),
            "systemError",
            400,
        ));
    }
    if !valid_id(post_id) {
        return Err(BusinessError::exception(
            "Invalid post id",
            "systemError",
            400,
        ));
    }
    let key = cache_key(workspace_id, inbox_id, post_id);
    if let Ok(cache) = CACHE.lock()
        && let Some((at, ttl, value)) = cache.get(&key)
        && at.elapsed() < *ttl
    {
        return Ok(value.clone());
    }
    let (value, degraded) = fetch(db, upstream, workspace_id, inbox_id, post_id, channel).await?;
    if let Ok(mut cache) = CACHE.lock() {
        cache.retain(|_, (at, ttl, _)| at.elapsed() < *ttl);
        let ttl = if degraded {
            DEGRADED_CACHE_TTL
        } else {
            CACHE_TTL
        };
        cache.insert(key, (Instant::now(), ttl, value.clone()));
    }
    Ok(value)
}
