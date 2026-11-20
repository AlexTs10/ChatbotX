//! `app/media/avatar/[token]/route.ts` and the `media-proxy.ts` helpers it
//! uses (`resolveMediaRequest`, `enqueueAvatarMirror`, `placeholderUrl`): the
//! public contact-avatar redirect behind a signed media token.

use std::sync::Arc;

use axum::Router;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get as route_get;
use chatbotx_business::channel_api::{ChannelHttp, LiveChannelHttp};
use chatbotx_business::contact::channel_profile::{
    contact_avatar, contact_inbox, resolve_fresh_contact_avatar_url,
};
use chatbotx_core::encryption::MediaTokenKind;
use chatbotx_core::utils::no_avatar_sentinel::{
    DEFAULT_AVATAR_PLACEHOLDER_PATH, is_no_avatar_sentinel_fresh, parse_no_avatar_sentinel,
};
use chatbotx_core::utils::storage::get_public_file_url;
use chatbotx_db::rusqlite::OptionalExtension as _;
use chatbotx_jobs::{JobOptions, QueueName, Retention};
use serde_json::json;

use crate::handlers::ApiState;
use crate::reflink_widget::{UNKNOWN_CLIENT_IP, check_guest_rate_limit_with, guest_client_ip};

/// `MEDIA_RATE_LIMIT_SCOPE`.
const MEDIA_RATE_LIMIT_SCOPE: &str = "media-proxy";
/// `MEDIA_IP_LIMIT` (`MEDIA_TOKEN_LIMIT` = the 20 per-session default).
const MEDIA_IP_LIMIT: i64 = 600;
/// `LOW_BULK_PRIORITY`.
const LOW_BULK_PRIORITY: u32 = 10;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn redirect(url: &str) -> Response {
    match HeaderValue::from_str(url) {
        Ok(location) => (StatusCode::FOUND, [(header::LOCATION, location)]).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// `loadServableWorkspace(...).servable`.
async fn servable_workspace(state: &ApiState, workspace_id: i64) -> bool {
    let row = state
        .db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    r#"SELECT "scheduledDeletionAt" FROM "Workspace" WHERE "id" = ?1"#,
                    [workspace_id],
                    |r| r.get::<_, Option<i64>>(0),
                )
                .optional()?)
        })
        .await;
    let workspace = row.unwrap_or_else(|err| {
        tracing::error!(err = %err, "Unable to load the media proxy workspace");
        None
    });
    chatbotx_business::workspace::lifecycle::resolve_freeze_reason(workspace, false).is_none()
}

/// `enqueueAvatarMirror` (fire and forget).
fn enqueue_avatar_mirror(
    state: &ApiState,
    workspace_id: i64,
    contact_inbox_id: i64,
    source_id: &str,
) {
    let Some(jobs) = state.jobs.clone() else {
        return;
    };
    let data = json!({ "type": "updateContactAvatar", "data": {
        "workspaceId": workspace_id.to_string(),
        "contactInboxId": contact_inbox_id.to_string(),
        "sourceId": source_id,
    }});
    let opts = JobOptions {
        job_id: Some(format!("update-avatar-{contact_inbox_id}")),
        priority: Some(LOW_BULK_PRIORITY),
        attempts: Some(2),
        remove_on_complete: Some(Retention::Remove),
        remove_on_fail: Some(Retention::KeepCount(100)),
        ..JobOptions::default()
    };
    tokio::spawn(async move {
        if let Err(err) = jobs
            .add(QueueName::Low, "updateContactAvatar", data, opts)
            .await
        {
            tracing::error!(%err, contact_inbox_id, "Failed to enqueue contact avatar mirror");
        }
    });
}

/// `GET /media/avatar/{token}` with an injectable provider client.
pub async fn serve(
    state: &ApiState,
    http: &dyn ChannelHttp,
    token: &str,
    headers: &HeaderMap,
) -> Response {
    // `resolveMediaRequest`.
    let claims = chatbotx_business::meta_catalog::process_encryption()
        .verify_media_token(token)
        .ok()
        .map(|payload| payload.claims)
        .filter(|claims| claims.kind == MediaTokenKind::Avatar);
    let Some(claims) = claims else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let (Ok(workspace_id), Ok(contact_inbox_id)) = (
        claims.workspace_id.parse::<i64>(),
        claims.ref_id.parse::<i64>(),
    ) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let client_ip = guest_client_ip(headers);
    let key = if client_ip == UNKNOWN_CLIENT_IP {
        claims.workspace_id.clone()
    } else {
        client_ip
    };
    let limit = check_guest_rate_limit_with(
        state,
        MEDIA_RATE_LIMIT_SCOPE,
        &key,
        Some(token),
        MEDIA_IP_LIMIT,
        now_ms(),
    );
    if limit.limited {
        let retry = HeaderValue::from(limit.retry_after);
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [(header::RETRY_AFTER, retry)],
        )
            .into_response();
    }
    if !servable_workspace(state, workspace_id).await {
        return StatusCode::GONE.into_response();
    }

    let urls = crate::handlers::contacts::tenant_urls(state);
    let placeholder = || {
        redirect(&format!(
            "{}{DEFAULT_AVATAR_PLACEHOLDER_PATH}",
            urls.app_url
        ))
    };
    let Ok(Some((contact_id, _, _, source_id))) = contact_inbox(&state.db, contact_inbox_id).await
    else {
        return placeholder();
    };
    let Ok(Some(avatar)) = contact_avatar(&state.db, workspace_id, contact_id).await else {
        return placeholder();
    };
    if let Some(avatar) = avatar.filter(|a| !a.is_empty()) {
        match parse_no_avatar_sentinel(&avatar) {
            None => {
                return match get_public_file_url(&avatar, &urls.storage_url) {
                    Ok(url) => redirect(&url),
                    Err(_) => placeholder(),
                };
            }
            Some(failed_at) if is_no_avatar_sentinel_fresh(failed_at, now_ms()) => {
                return placeholder();
            }
            Some(_) => {}
        }
    }
    let fresh =
        resolve_fresh_contact_avatar_url(&state.db, http, workspace_id, contact_inbox_id).await;
    enqueue_avatar_mirror(state, workspace_id, contact_inbox_id, &source_id);
    match fresh {
        Ok(Some(url)) => redirect(&url),
        Ok(None) => placeholder(),
        Err(message) => {
            // ponytail: every integration error degrades quietly; the original
            // logs only non-terminal ones.
            tracing::warn!(%message, contact_inbox_id, "Failed to resolve fresh contact avatar URL; serving placeholder");
            placeholder()
        }
    }
}

/// The `/media/avatar/{token}` route.
pub fn router(state: Arc<ApiState>) -> Router {
    Router::new()
        .route(
            "/media/avatar/{token}",
            route_get(
                |State(state): State<Arc<ApiState>>,
                 Path(token): Path<String>,
                 headers: HeaderMap| async move {
                    serve(&state, &LiveChannelHttp, &token, &headers).await
                },
            ),
        )
        .with_state(state)
}
