//! The Graph reads behind the post pickers and the Instagram Live
//! subscription: `listInstagramLoginMedia` / `listInstagramFacebookMedia`
//! (`features/ig-comments/lib/instagram-media.ts`), the story twins
//! (`features/ig-stories/lib/instagram-stories.ts`) and
//! `ensureLiveCommentsSubscriptionForAutomation`
//! (`features/ig-comments/lib/ensure-live-comments-subscription.ts`).
//!
//! Fixed Meta hosts only (no caller-chosen URL); one failing account is
//! logged and left out (`collectSettled`).

use chatbotx_db::Db;
use serde_json::{Map, Value, json};

use crate::connection::credentials::{Upstream, UpstreamRequest};
use crate::errors::{BusinessError, Result};
use crate::tx;

const MEDIA_FIELDS: &str =
    "id,caption,media_product_type,media_type,media_url,thumbnail_url,timestamp,permalink";
const STORY_FIELDS: &str = "id,caption,media_type,media_url,thumbnail_url,timestamp,permalink";
/// `INSTAGRAM_SUBSCRIBE_FIELDS`.
const SUBSCRIBE_FIELDS: &str = "messages,messaging_postbacks,messaging_optins,messaging_seen,messaging_referral,comments,live_comments";

/// One connected Instagram account.
#[derive(Debug, Clone)]
struct Account {
    ig_id: String,
    name: String,
    inbox_id: Option<i64>,
    token: String,
    version: Option<String>,
}

async fn accounts(
    db: &Db,
    workspace_id: i64,
    row_type: &'static str,
    inbox_id: Option<i64>,
) -> Result<Vec<Account>> {
    tx::read(db, move |conn| {
        let mut stmt = conn
            .prepare(
                r#"SELECT "igId", "name", "inboxId", "auth" FROM "IntegrationInstagram"
                   WHERE "workspaceId" = ?1 AND "type" = ?2 ORDER BY "id""#,
            )
            .map_err(|e| BusinessError::Repo(e.into()))?;
        let rows = stmt
            .query_map(chatbotx_db::rusqlite::params![workspace_id, row_type], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })
            .map_err(|e| BusinessError::Repo(e.into()))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| BusinessError::Repo(e.into()))?;
        Ok(rows
            .into_iter()
            // `filterIntegrationsByAccount`.
            .filter(|(_, _, inbox, _)| inbox_id.is_none() || *inbox == inbox_id)
            .map(|(ig_id, name, inbox, auth)| {
                let auth: Value = serde_json::from_str(&auth).unwrap_or(Value::Null);
                Account {
                    ig_id,
                    name,
                    inbox_id: inbox,
                    token: auth["tokens"]["accessToken"].as_str().unwrap_or_default().to_string(),
                    version: auth["metadata"]["version"].as_str().map(str::to_string),
                }
            })
            .collect())
    })
    .await
}

/// `resolvePostPreviewImage` (media) / `media_url ?? thumbnail_url` (stories).
fn picture(item: &Value, stories: bool) -> Option<Value> {
    let media = item.get("media_url").filter(|v| !v.is_null()).cloned();
    let thumb = item.get("thumbnail_url").filter(|v| !v.is_null()).cloned();
    let video = item["media_type"]
        .as_str()
        .is_some_and(|t| t.eq_ignore_ascii_case("VIDEO"));
    if video && !stories {
        return thumb;
    }
    media.or(thumb)
}

/// `{ posts | stories, pages }` of the `instagram` (Instagram Login) or
/// `instagramFacebook` variant.
pub async fn list_instagram_media<U: Upstream>(
    db: &Db,
    upstream: &U,
    workspace_id: i64,
    variant: &str,
    inbox_id: Option<i64>,
    stories: bool,
) -> Result<Value> {
    let facebook = variant == "instagramFacebook";
    let list = accounts(db, workspace_id, if facebook { "facebook" } else { "instagram" }, inbox_id).await?;
    let pages: Vec<Value> = list
        .iter()
        .map(|a| json!({ "id": a.ig_id, "name": a.name }))
        .collect();
    let mut items = Vec::new();
    for account in &list {
        let edge = if stories { "stories" } else { "media" };
        let url = if facebook {
            let version = account.version.as_deref().unwrap_or("v23.0");
            format!("https://graph.facebook.com/{version}/{}/{edge}", account.ig_id)
        } else {
            let version = account.version.as_deref().unwrap_or("v22.0");
            format!("https://graph.instagram.com/{version}/me/{edge}")
        };
        let fields = if stories { STORY_FIELDS } else { MEDIA_FIELDS };
        let url = format!("{url}?fields={}&limit=100", fields.replace(',', "%2C"));
        let reply = upstream
            .send(UpstreamRequest {
                method: "GET",
                url,
                headers: vec![("Authorization".into(), format!("Bearer {}", account.token))],
                body: None,
            })
            .await;
        let body: Value = match reply {
            Ok((status, body)) if (200..300).contains(&status) => {
                serde_json::from_str(&body).unwrap_or(Value::Null)
            }
            other => {
                tracing::warn!(?other, ig_id = %account.ig_id, "failed to list Instagram media for an integration");
                continue;
            }
        };
        for item in body["data"].as_array().into_iter().flatten() {
            let mut out = Map::new();
            out.insert("id".into(), item["id"].clone());
            let mut put = |key: &str, value: Option<Value>| {
                if let Some(value) = value.filter(|v| !v.is_null()) {
                    out.insert(key.into(), value);
                }
            };
            put("message", item.get("caption").cloned());
            put("full_picture", picture(item, stories));
            put("created_time", item.get("timestamp").cloned());
            put("permalink_url", item.get("permalink").cloned());
            if !stories {
                put("media_product_type", item.get("media_product_type").cloned());
            }
            out.insert("accountId".into(), json!(account.ig_id));
            out.insert(
                "inboxId".into(),
                account.inbox_id.map_or(Value::Null, |id| json!(id.to_string())),
            );
            items.push(Value::Object(out));
        }
    }
    let key = if stories { "stories" } else { "posts" };
    Ok(json!({ key: items, "pages": pages }))
}

/// `ensureLiveCommentsSubscriptionForAutomation`: an `instagram` Live
/// automation re-subscribes every Instagram Login account (best effort).
pub async fn ensure_live_subscription<U: Upstream>(
    db: &Db,
    upstream: &U,
    workspace_id: i64,
    kind: &str,
    post: Option<&Value>,
) {
    let live = post.and_then(|p| p["type"].as_str()) == Some("live");
    if kind != "instagram" || !live {
        return;
    }
    let list = match accounts(db, workspace_id, "instagram", None).await {
        Ok(list) => list,
        Err(error) => {
            tracing::warn!(%error, "live_comments subscription: account read failed");
            return;
        }
    };
    for account in list {
        let version = account.version.as_deref().unwrap_or("v22.0");
        let reply = upstream
            .send(UpstreamRequest {
                method: "POST",
                url: format!("https://graph.instagram.com/{version}/{}/subscribed_apps", account.ig_id),
                headers: vec![
                    ("Authorization".into(), format!("Bearer {}", account.token)),
                    ("Content-Type".into(), "application/json".into()),
                ],
                body: Some(json!({ "subscribed_fields": SUBSCRIBE_FIELDS }).to_string()),
            })
            .await;
        let ok = matches!(&reply, Ok((status, body)) if (200..300).contains(status)
            && serde_json::from_str::<Value>(body).ok().and_then(|b| b["success"].as_bool()) == Some(true));
        if !ok {
            tracing::warn!(ig_id = %account.ig_id, "failed to subscribe Instagram account to live_comments");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_media_shows_its_thumbnail_but_a_video_story_its_url() {
        let video = json!({ "media_type": "VIDEO", "media_url": "v.mp4", "thumbnail_url": "t.jpg" });
        assert_eq!(picture(&video, false), Some(json!("t.jpg")));
        assert_eq!(picture(&video, true), Some(json!("v.mp4")));
        assert_eq!(picture(&json!({ "media_type": "VIDEO", "media_url": "v.mp4" }), false), None);
        assert_eq!(picture(&json!({ "thumbnail_url": "t.jpg" }), false), Some(json!("t.jpg")));
    }
}
