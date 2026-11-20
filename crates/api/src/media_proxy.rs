//! `apps/builder/src/app/media/attachment/[token]/route.ts` and
//! `media/media-proxy.ts`: the signed attachment proxy. A mirrored object
//! redirects to a presigned storage URL, a pending one to its provider URL
//! (or the processing placeholder) while a mirror job runs, a failed one to
//! the unavailable placeholder.
//!
//! ponytail: no `resolveFreshMediaUrl` (a Graph re-derivation of the provider
//! URL). A pending row with a stored Meta CDN URL redirects to it; any other
//! pending row (a `wa-media:` id, a retry) shows the processing placeholder
//! and the mirror job fetches it. A retry on a mirrored row queues a restore
//! and shows the unavailable placeholder. Port the Graph lookup when
//! expired-URL renders matter.

use std::sync::Arc;

use axum::Router;
use axum::extract::{Path, RawQuery, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use chatbotx_business::media_url::{
    HYDRATION_CHANNELS, is_failed_origin_path, is_pending_origin_path, verify_media_token,
};
use chatbotx_business::tx;
use chatbotx_core::encryption::MediaTokenKind;
use chatbotx_core::utils::live_media::{META_MEDIA_HOSTS, is_url_on_hosts};
use chatbotx_core::utils::request::{
    RequestParts, force_public_https, get_public_origin_from_request,
};
use chatbotx_db::repo::message::{AttachmentLookupRow, FindAttachmentByIdParams};
use chatbotx_jobs::{Backoff, Dedup, JobOptions, QueueName, Retention};
use serde_json::{Value, json};

use crate::handlers::ApiState;
use crate::reflink_widget::{UNKNOWN_CLIENT_IP, check_guest_rate_limit_with, guest_client_ip};

const MEDIA_RATE_LIMIT_SCOPE: &str = "media-proxy";
const MEDIA_IP_LIMIT: i64 = 600;
const JOB_NAME: &str = "coexistAttachmentDownload";
/// `LOW_BULK_PRIORITY`.
const LOW_BULK_PRIORITY: u32 = 10;
const RETRY_BACKOFF: Backoff = Backoff::Exponential(30_000);
const RESTORE_DEDUP_TTL_MS: u64 = 60 * 60 * 1000;

/// `GET /media/attachment/{token}`.
pub fn router(state: Arc<ApiState>) -> Router {
    Router::new()
        .route("/media/attachment/{token}", get(handle))
        .with_state(state)
}

fn status(code: StatusCode) -> Response {
    code.into_response()
}

fn redirect(location: &str) -> Response {
    (
        StatusCode::FOUND,
        [(header::LOCATION, location.to_string())],
    )
        .into_response()
}

/// `placeholderUrl`: `/media/<file>` on the public origin.
fn placeholder(headers: &HeaderMap, file: &str) -> Response {
    let parts = RequestParts {
        url: "http://localhost/",
        headers,
    };
    let origin =
        get_public_origin_from_request(&parts, force_public_https(&chatbotx_core::env::ProcessEnv));
    redirect(&format!("{origin}/media/{file}"))
}

async fn servable(api: &ApiState, workspace_id: i64) -> bool {
    tx::read(&api.db, move |c| {
        Ok(c.query_row(
            r#"SELECT count(*) FROM "Workspace" WHERE "id" = ?1 AND "scheduledDeletionAt" IS NULL"#,
            [workspace_id],
            |r| r.get::<_, i64>(0),
        )
        .map_err(chatbotx_db::repo::Error::from)?
            > 0)
    })
    .await
    .unwrap_or(false)
}

/// The channel of the attachment's message (the job's `channel`).
async fn channel_of(api: &ApiState, workspace_id: i64, message_id: i64) -> Option<String> {
    tx::read(&api.db, move |c| {
        Ok(c.query_row(
            r#"SELECT ci."channel" FROM "Message" m
               JOIN "ContactInbox" ci ON ci."id" = m."contactInboxId"
               WHERE m."id" = ?1 AND m."workspaceId" = ?2"#,
            [message_id, workspace_id],
            |r| r.get::<_, String>(0),
        )
        .map_err(chatbotx_db::repo::Error::from)?)
    })
    .await
    .ok()
}

/// `addAttachmentJob` (fire and forget: a queue error is only logged).
async fn add_job(api: &ApiState, data: Value, opts: JobOptions) {
    let Some(jobs) = api.jobs.as_ref() else {
        return;
    };
    let attachment_id = data["attachmentId"].clone();
    let body = json!({ "type": JOB_NAME, "data": data });
    let opts = JobOptions {
        priority: Some(LOW_BULK_PRIORITY),
        ..opts
    };
    if let Err(error) = jobs.add(QueueName::Low, JOB_NAME, body, opts).await {
        tracing::error!(%error, %attachment_id, "Failed to enqueue attachment media job");
    }
}

fn job_data(workspace_id: i64, row: &AttachmentLookupRow, channel: &str, restore: bool) -> Value {
    let mut data = json!({
        "attachmentId": row.id.to_string(),
        "workspaceId": workspace_id.to_string(),
        "channel": channel,
        "messageCreatedAt": row.message_created_at,
    });
    if restore {
        data["restore"] = json!(true);
    }
    data
}

/// `enqueueAttachmentMirror` / `enqueueAttachmentRestore` (only the
/// hydration channels have a mirror job).
async fn enqueue(api: &ApiState, workspace_id: i64, row: &AttachmentLookupRow, restore: bool) {
    let Some(channel) = channel_of(api, workspace_id, row.message_id).await else {
        return;
    };
    if !HYDRATION_CHANNELS.contains(&channel.as_str()) {
        return;
    }
    let data = job_data(workspace_id, row, &channel, restore);
    let opts = if restore {
        JobOptions {
            dedup: Some(Dedup {
                id: format!("media-restore-{}", row.id),
                ttl_ms: Some(RESTORE_DEDUP_TTL_MS),
                ..Dedup::default()
            }),
            attempts: Some(3),
            backoff: Some(RETRY_BACKOFF),
            remove_on_complete: Some(Retention::Remove),
            remove_on_fail: Some(Retention::Remove),
            ..JobOptions::default()
        }
    } else {
        JobOptions {
            job_id: Some(format!("media-message-{}", row.message_id)),
            attempts: Some(5),
            backoff: Some(RETRY_BACKOFF),
            remove_on_complete: Some(Retention::Remove),
            remove_on_fail: Some(Retention::KeepCount(100)),
            ..JobOptions::default()
        }
    };
    add_job(api, data, opts).await;
}

async fn handle(
    State(api): State<Arc<ApiState>>,
    Path(token): Path<String>,
    RawQuery(raw): RawQuery,
    headers: HeaderMap,
) -> Response {
    // `resolveMediaRequest`.
    let Some(claims) = verify_media_token(&token).filter(|c| c.kind == MediaTokenKind::Attachment)
    else {
        return status(StatusCode::NOT_FOUND);
    };
    let (Ok(workspace_id), Ok(attachment_id)) = (
        claims.workspace_id.parse::<i64>(),
        claims.ref_id.parse::<i64>(),
    ) else {
        return status(StatusCode::NOT_FOUND);
    };
    // `resolveGuestRateLimitKey`: the workspace when the IP is unknown.
    let ip = guest_client_ip(&headers);
    let key = if ip == UNKNOWN_CLIENT_IP {
        claims.workspace_id.clone()
    } else {
        ip
    };
    let now = crate::reflink_widget::now_ms();
    let limit = check_guest_rate_limit_with(
        &api,
        MEDIA_RATE_LIMIT_SCOPE,
        &key,
        Some(&token),
        MEDIA_IP_LIMIT,
        now,
    );
    if limit.limited {
        let mut response = status(StatusCode::TOO_MANY_REQUESTS);
        response
            .headers_mut()
            .insert(header::RETRY_AFTER, HeaderValue::from(limit.retry_after));
        return response;
    }
    if !servable(&api, workspace_id).await {
        return status(StatusCode::GONE);
    }

    let is_retry = url::form_urlencoded::parse(raw.unwrap_or_default().as_bytes())
        .any(|(k, v)| k == "retry" && v == "1");
    let params = FindAttachmentByIdParams {
        id: attachment_id,
        message_created_at: claims.message_created_at,
        workspace_id,
    };
    let lookup = tx::read(&api.db, move |c| {
        Ok(chatbotx_db::repo::message::find_attachment_by_id(
            c, &params,
        )?)
    })
    .await;
    let row = match lookup {
        Ok(Some(row)) => row,
        Ok(None) => return status(StatusCode::NOT_FOUND),
        Err(error) => {
            tracing::error!(%error, attachment_id, "Failed to look up attachment");
            return status(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };

    if is_failed_origin_path(&row.origin_path) {
        return placeholder(&headers, "unavailable.svg");
    }
    if !is_pending_origin_path(&row.origin_path) {
        if is_retry {
            enqueue(&api, workspace_id, &row, true).await;
            return placeholder(&headers, "unavailable.svg");
        }
        let presigned = match api.files.as_deref() {
            Some(files) => files.get_presigned_download(&row.origin_path, None).await,
            None => return placeholder(&headers, "unavailable.svg"),
        };
        return match presigned {
            Ok(url) => redirect(&url),
            Err(error) => {
                tracing::error!(%error, attachment_id, "Failed to presign mirrored attachment");
                placeholder(&headers, "unavailable.svg")
            }
        };
    }

    enqueue(&api, workspace_id, &row, false).await;
    if !is_retry && is_url_on_hosts(&row.origin_path, Some(META_MEDIA_HOSTS)) {
        return redirect(&row.origin_path);
    }
    placeholder(&headers, "processing.svg")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_data_shape() {
        let row = AttachmentLookupRow {
            id: 5,
            message_id: 9,
            message_created_at: 1_700,
            source_id: None,
            origin_path: "wa-media:1".into(),
            mime_type: "image/png".into(),
            size: 1,
            created_at: 1,
            keep_original: None,
        };
        let data = job_data(2, &row, "whatsapp", true);
        assert_eq!(
            data,
            json!({ "attachmentId": "5", "workspaceId": "2", "channel": "whatsapp", "messageCreatedAt": 1_700, "restore": true })
        );
    }
}
