//! `resolveFreshMediaUrl` (`channel-registry/media-hydration.ts`, Q.P7-V5):
//! the attachment proxy re-derives a Messenger / Instagram attachment's
//! provider URL from the Graph message (`getMessageMediaUrls`) when the stored
//! webhook URL has expired (`?retry=1`) or a mirrored object was evicted.
//! WhatsApp and the other channels expose no fresh URL (`None`).

use chatbotx_core::utils::live_media::{META_MEDIA_HOSTS, is_url_on_hosts};
use chatbotx_db::Db;
use chatbotx_db::repo::message::{self, FindAttachmentByIdParams, FindMessageByIdParams};
use chatbotx_db::repo::models::Attachment;
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use chatbotx_files::guess_file_type_from_mime_type;
use serde_json::Value;

use crate::connection::credentials::{Upstream, UpstreamRequest};
use crate::media_url::{is_failed_origin_path, is_pending_origin_path};

pub const FACEBOOK_API_URL: &str = "https://graph.facebook.com";
const INSTAGRAM_API_URL: &str = "https://graph.instagram.com";
/// `MESSAGE_MEDIA_FIELDS` of the Messenger / Instagram `apis/sync.ts`.
const MESSENGER_MEDIA_FIELDS: &str =
    "attachments{id,name,mime_type,size,image_data,video_data,file_url}";
const INSTAGRAM_MEDIA_FIELDS: &str =
    "attachments{id,name,mime_type,size,payload,image_data,video_data,file_url}";

/// `resolveFreshMediaUrl` input.
#[derive(Debug, Clone, Copy, Default)]
pub struct FreshMediaRequest {
    pub attachment_id: i64,
    pub workspace_id: i64,
    pub message_created_at: Option<i64>,
    /// Also re-derive a mirrored row (its stored object is gone).
    pub allow_mirrored: bool,
    /// The stored webhook URL already failed to load: ask the provider.
    pub skip_stored_url: bool,
}

/// `ResolvedFreshMedia`.
#[derive(Debug, Clone, PartialEq)]
pub struct FreshMedia {
    pub url: String,
    pub channel: String,
    pub integration_id: i64,
}

/// Why no URL could be derived.
#[derive(Debug)]
pub enum FreshMediaError {
    /// `TerminalMediaError`: the attachment can never be served again.
    Terminal(String),
    Failed(String),
}

impl std::fmt::Display for FreshMediaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FreshMediaError::Terminal(m) | FreshMediaError::Failed(m) => f.write_str(m),
        }
    }
}

/// One entry of `getMessageMediaUrls`.
struct Fresh {
    source_id: String,
    url: String,
    mime_type: Option<String>,
}

/// The message's attachments (by id), its Graph id, the contact inbox's
/// channel and the integration (`id`, auth JSON, native Instagram).
struct Graph {
    attachments: Vec<Attachment>,
    message_source_id: Option<String>,
    channel: String,
    integration: Option<(i64, Value, bool)>,
}

fn failed(error: impl std::fmt::Display) -> FreshMediaError {
    FreshMediaError::Failed(error.to_string())
}

async fn load(db: &Db, input: FreshMediaRequest) -> Result<Option<Graph>, FreshMediaError> {
    chatbotx_db::repo::read(db, move |c| {
        let ws = input.workspace_id;
        let Some(row) = message::find_attachment_by_id(
            c,
            &FindAttachmentByIdParams {
                id: input.attachment_id,
                message_created_at: input.message_created_at,
                workspace_id: ws,
            },
        )?
        else {
            return Ok(None);
        };
        if is_failed_origin_path(&row.origin_path) {
            return Ok(Some(Err(())));
        }
        if !(input.allow_mirrored || is_pending_origin_path(&row.origin_path)) {
            return Ok(None);
        }
        let Some(found) = message::find_by_id(
            c,
            &FindMessageByIdParams {
                conversation_id: None,
                created_at: row.message_created_at,
                id: row.message_id,
                workspace_id: ws,
            },
        )?
        else {
            return Ok(None);
        };
        let Some((channel, inbox_id)) = c
            .query_row(
                r#"SELECT ci."channel", ci."inboxId" FROM "ContactInbox" ci
                   JOIN "Inbox" i ON i."id" = ci."inboxId" AND i."workspaceId" = ?2
                   WHERE ci."id" = ?1"#,
                params![found.message.contact_inbox_id, ws],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
            )
            .optional()?
        else {
            return Ok(None);
        };
        let sql = match channel.as_str() {
            "messenger" => {
                r#"SELECT "id", "auth", 0 FROM "IntegrationMessenger" WHERE "inboxId" = ?1 AND "workspaceId" = ?2 LIMIT 1"#
            }
            "instagram" => {
                r#"SELECT "id", "auth", "type" = 'instagram' FROM "IntegrationInstagram" WHERE "inboxId" = ?1 AND "workspaceId" = ?2 LIMIT 1"#
            }
            _ => return Ok(None),
        };
        let integration = c
            .query_row(sql, params![inbox_id, ws], |r| {
                let auth: Option<String> = r.get(1)?;
                Ok((
                    r.get::<_, i64>(0)?,
                    auth.and_then(|a| serde_json::from_str(&a).ok())
                        .unwrap_or(Value::Null),
                    r.get::<_, Option<bool>>(2)?.unwrap_or(false),
                ))
            })
            .optional()?;
        Ok(Some(Ok(Graph {
            attachments: found.attachments,
            message_source_id: found.message.source_id,
            channel,
            integration,
        })))
    })
    .await
    .map_err(failed)?
    .map(|graph| {
        graph.map_err(|()| {
            FreshMediaError::Terminal(format!(
                "Attachment {} is permanently unavailable",
                input.attachment_id
            ))
        })
    })
    .transpose()
}

/// `resolveGraphMedia`: the message's attachments that carry a URL and an id.
async fn graph_media(
    upstream: &impl Upstream,
    graph_url: &str,
    channel: &str,
    auth: &Value,
    native_instagram: bool,
    message_id: &str,
) -> Result<Vec<Fresh>, FreshMediaError> {
    let token = auth
        .pointer("/tokens/accessToken")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
        .ok_or_else(|| {
            failed(format!(
                "[media-hydration] Missing access token for {channel}"
            ))
        })?;
    let version = auth
        .pointer("/metadata/version")
        .and_then(Value::as_str)
        .unwrap_or(if native_instagram { "v22.0" } else { "v23.0" });
    // ponytail: a non-default `graph_url` (a test server) also stands in for
    // graph.instagram.com.
    let base = if native_instagram && graph_url == FACEBOOK_API_URL {
        INSTAGRAM_API_URL
    } else {
        graph_url
    };
    let instagram = channel == "instagram";
    let mut url = url::Url::parse(&format!("{base}/{version}/{message_id}")).map_err(failed)?;
    url.query_pairs_mut().append_pair(
        "fields",
        if instagram {
            INSTAGRAM_MEDIA_FIELDS
        } else {
            MESSENGER_MEDIA_FIELDS
        },
    );
    let (status, body) = upstream
        .send(UpstreamRequest {
            method: "GET",
            url: url.to_string(),
            headers: vec![("Authorization".into(), format!("Bearer {token}"))],
            body: None,
        })
        .await
        .map_err(failed)?;
    if !(200..300).contains(&status) {
        return Err(failed(format!(
            "[media-hydration] Graph message lookup failed ({status})"
        )));
    }
    let parsed: Value = serde_json::from_str(&body).map_err(failed)?;
    Ok(parsed["attachments"]["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| {
            let url = instagram
                .then(|| a["payload"]["url"].as_str())
                .flatten()
                .or_else(|| a["image_data"]["url"].as_str())
                .or_else(|| a["video_data"]["url"].as_str())
                .or_else(|| a["file_url"].as_str())?;
            Some(Fresh {
                source_id: a["id"].as_str()?.to_owned(),
                url: url.to_owned(),
                mime_type: a["mime_type"].as_str().map(str::to_owned),
            })
        })
        .collect())
}

/// `resolveFreshMediaUrl`: `Ok(None)` = the channel cannot expose a fresh URL,
/// or the provider no longer returns this attachment.
/// `graph_url` = the Graph base (`https://graph.facebook.com`).
pub async fn resolve_fresh_media_url(
    db: &Db,
    upstream: &impl Upstream,
    graph_url: &str,
    input: FreshMediaRequest,
) -> Result<Option<FreshMedia>, FreshMediaError> {
    let Some(mut graph) = load(db, input).await? else {
        return Ok(None);
    };
    let Some((integration_id, auth, native_instagram)) = graph.integration.take() else {
        return Ok(None);
    };
    let Some(message_id) = graph.message_source_id.as_deref() else {
        return Err(FreshMediaError::Terminal(format!(
            "Message source id is missing for attachment {}",
            input.attachment_id
        )));
    };
    // `compareAttachmentIds`.
    graph.attachments.sort_by_key(|a| a.id);
    let attachments = &graph.attachments;
    let Some(index) = attachments.iter().position(|a| a.id == input.attachment_id) else {
        return Ok(None);
    };
    let requested = &attachments[index];
    let found = |url: String| FreshMedia {
        url,
        channel: graph.channel.clone(),
        integration_id,
    };
    // `isLiveWebhookRow`: the webhook's own URL is served as it is.
    if !input.skip_stored_url
        && requested.source_id.is_none()
        && is_url_on_hosts(&requested.origin_path, Some(META_MEDIA_HOSTS))
    {
        return Ok(Some(found(requested.origin_path.clone())));
    }
    let media = graph_media(
        upstream,
        graph_url,
        &graph.channel,
        &auth,
        native_instagram,
        message_id,
    )
    .await?;
    let mirrored = !(is_failed_origin_path(&requested.origin_path)
        || is_pending_origin_path(&requested.origin_path));
    let resolved = match &requested.source_id {
        Some(source_id) => media.iter().find(|m| m.source_id == *source_id),
        // `pairByPosition`: equal counts and the same kind.
        None => media.get(index).filter(|m| {
            media.len() == attachments.len()
                && guess_file_type_from_mime_type(m.mime_type.as_deref().unwrap_or_default())
                    .as_str()
                    == requested.file_type
        }),
    }
    // A mirrored single-attachment message pairs by position without an id.
    .or_else(|| (mirrored && attachments.len() == 1 && media.len() == 1).then(|| &media[0]));
    Ok(resolved.map(|m| found(m.url.clone())))
}
