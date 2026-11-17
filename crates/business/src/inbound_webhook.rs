//! `packages/business/src/inbound-webhook/service.ts`: CRUD, settings,
//! token, request log and intake writes.
//!
//! ponytail: no token cache (the intake reads one indexed row per delivery)
//! and no config lock: a settings change reads, checks and writes in one
//! SQLite write job, which is already serial.
//! ponytail: `update` (config save), `previewMapping` and the worker
//! processing are not ported yet (see `conversion/specs/Q.D13.md`).

use chatbotx_core::utils::channel::ChannelType;
use chatbotx_core::utils::inbound_webhook_channel::{
    InboxIdentifierKind, get_inbox_identifier_strategy,
};
use chatbotx_db::Db;
use chatbotx_db::repo::inbound_webhook::{
    self as repo_iw, InboundWebhookIntake, InboundWebhookListRow, InboundWebhookRequestListRow,
    ListInboundWebhookRequestsInput,
};
use chatbotx_db::repo::models::{InboundWebhook, InboundWebhookRequest};
use chatbotx_db::repo::patch::{Patch, json as json_value};
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, types::Value};
use serde_json::Value as Json;

use crate::errors::{BusinessError, Result};
use crate::tx;
use crate::workspace_api_token::credentials::generate_inbound_webhook_token;

/// `INBOUND_WEBHOOK_LIMITS.maxWebhooksPerWorkspace`.
pub const MAX_WEBHOOKS_PER_WORKSPACE: i64 = 100;
/// `INBOUND_WEBHOOK_LIMITS.maxPayloadBytes`.
pub const MAX_PAYLOAD_BYTES: usize = 256 * 1024;
/// `INBOUND_WEBHOOK_LIMITS.maxIdempotencyKeyLength`.
pub const MAX_IDEMPOTENCY_KEY_LENGTH: usize = 255;
/// `INBOUND_WEBHOOK_LIMITS.maxUserAgentLength`.
pub const MAX_USER_AGENT_LENGTH: usize = 512;
/// `INBOUND_WEBHOOK_LIMITS.maxNameLength`.
pub const MAX_NAME_LENGTH: usize = 255;
/// `INBOUND_WEBHOOK_TOKEN_PREFIX`.
pub const TOKEN_PREFIX: &str = "iwh_";
/// `inboundWebhookRequestStatuses`.
pub const REQUEST_STATUSES: &[&str] = &[
    "received",
    "processing",
    "processed",
    "completedWithErrors",
    "skipped",
    "failed",
];

const NOT_FOUND: &str = "Inbound webhook not found";
const REQUEST_NOT_FOUND: &str = "Inbound webhook request not found";

fn not_found() -> BusinessError {
    BusinessError::not_found(NOT_FOUND)
}

fn find_in(c: &Connection, workspace_id: i64, id: i64) -> Result<InboundWebhook> {
    repo_iw::find_inbound_webhook_by_id(c, id, workspace_id)?.ok_or_else(not_found)
}

/// `create`: an inactive, empty webhook with a new token (max 100 per workspace).
pub async fn create(db: &Db, workspace_id: i64, name: String) -> Result<InboundWebhook> {
    tx::write(db, move |c| {
        if repo_iw::count_inbound_webhooks(c, workspace_id)? >= MAX_WEBHOOKS_PER_WORKSPACE {
            let mut error = BusinessError::validation("_", "validation.maxItemsReached");
            if let BusinessError::Exception { data, .. } = &mut error {
                *data = Some(serde_json::json!({
                    "max": MAX_WEBHOOKS_PER_WORKSPACE,
                    "feature": "inboundWebhooks",
                }));
            }
            return Err(error);
        }
        let values: Patch = vec![
            ("workspaceId", Value::Integer(workspace_id)),
            ("name", Value::Text(name)),
            ("active", Value::Integer(0)),
            ("token", Value::Text(generate_inbound_webhook_token())),
            ("inboxResolution", Value::Null),
            ("identifiers", Value::Text("[]".into())),
            ("fieldMappings", Value::Text("[]".into())),
            ("actions", Value::Text("[]".into())),
            ("sampleJson", Value::Null),
            ("requestCount", Value::Integer(0)),
        ];
        Ok(repo_iw::insert_inbound_webhook(c, &values)?)
    })
    .await
}

/// `list`: `(rows, pageCount)`, newest first.
pub async fn list(
    db: &Db,
    workspace_id: i64,
    page: i64,
    per_page: i64,
) -> Result<(Vec<InboundWebhookListRow>, i64)> {
    let per_page = per_page.max(1);
    let offset = (page.max(1) - 1) * per_page;
    let out = tx::read(db, move |c| {
        Ok(repo_iw::list_inbound_webhooks_paginated(
            c,
            workspace_id,
            per_page,
            offset,
        )?)
    })
    .await?;
    Ok((out.rows, (out.total + per_page - 1) / per_page))
}

/// `findOrFail`.
pub async fn find_or_fail(db: &Db, workspace_id: i64, id: i64) -> Result<InboundWebhook> {
    tx::read(db, move |c| find_in(c, workspace_id, id)).await
}

/// `findByToken` (unscoped: the token is the capability).
pub async fn find_by_token(db: &Db, token: String) -> Result<Option<InboundWebhookIntake>> {
    tx::read(db, move |c| {
        Ok(repo_iw::find_inbound_webhook_intake_by_token(c, &token)?)
    })
    .await
}

/// `resolveEffectiveChannel` + `checkInboxResolution` on a stored resolution.
fn inbox_resolution_issue(
    c: &Connection,
    workspace_id: i64,
    resolution: &Json,
) -> Result<Option<&'static str>> {
    if resolution.get("mode").and_then(Json::as_str) == Some("payload") {
        let channel = resolution
            .get("channel")
            .and_then(Json::as_str)
            .and_then(ChannelType::parse);
        let kind = resolution
            .get("identifierKind")
            .and_then(Json::as_str)
            .and_then(InboxIdentifierKind::parse);
        let supported = matches!((channel, kind), (Some(ch), Some(k)) if get_inbox_identifier_strategy(ch, k).is_some());
        return Ok((!supported).then_some("unsupportedInboxIdentifier"));
    }
    let inbox_id = resolution
        .get("inboxId")
        .and_then(|v| {
            v.as_str()
                .map(str::to_owned)
                .or_else(|| v.as_i64().map(|n| n.to_string()))
        })
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(-1);
    let status: Option<String> = c
        .query_row(
            r#"SELECT "status" FROM "Inbox" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            [inbox_id, workspace_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(chatbotx_db::repo::Error::from)?;
    Ok(match status {
        None => Some("inboxNotFound"),
        Some(s) if s != "connected" => Some("inboxDisconnected"),
        Some(_) => None,
    })
}

/// `assertConfigValid`: the first issue of `validateInboundWebhookConfig`.
fn assert_config_valid(c: &Connection, workspace_id: i64, row: &InboundWebhook) -> Result<()> {
    let empty = |v: &Json| v.as_array().is_none_or(Vec::is_empty);
    let resolution = row
        .inbox_resolution
        .as_ref()
        .map(|j| &j.0)
        .filter(|v| !v.is_null());
    let issue = if resolution.is_none() {
        Some(("inboxResolution", "inboxRequired"))
    } else if empty(&row.identifiers.0) {
        Some(("identifiers", "identifierRequired"))
    } else if empty(&row.field_mappings.0) {
        Some(("fieldMappings", "fieldMappingRequired"))
    } else {
        resolution
            .map(|r| inbox_resolution_issue(c, workspace_id, r))
            .transpose()?
            .flatten()
            .map(|code| ("inboxResolution", code))
    };
    match issue {
        Some((field, code)) => Err(BusinessError::validation(
            field,
            format!("validation.inboundWebhook.{code}"),
        )),
        None => Ok(()),
    }
}

/// `updateSettings`: `{}` returns the row unchanged; activating an inactive
/// webhook checks its stored config first.
pub async fn update_settings(
    db: &Db,
    workspace_id: i64,
    id: i64,
    name: Option<String>,
    active: Option<bool>,
) -> Result<InboundWebhook> {
    tx::write(db, move |c| {
        let existing = find_in(c, workspace_id, id)?;
        if name.is_none() && active.is_none() {
            return Ok(existing);
        }
        if active == Some(true) && !existing.active {
            assert_config_valid(c, workspace_id, &existing)?;
        }
        let mut set: Patch = Vec::new();
        if let Some(name) = name {
            set.push(("name", Value::Text(name)));
        }
        if let Some(active) = active {
            set.push(("active", Value::Integer(i64::from(active))));
        }
        repo_iw::update_inbound_webhook(c, id, workspace_id, &set)?.ok_or_else(not_found)
    })
    .await
}

/// `regenerateToken`: the old URL stops working at once.
pub async fn regenerate_token(db: &Db, workspace_id: i64, id: i64) -> Result<InboundWebhook> {
    tx::write(db, move |c| {
        let set: Patch = vec![("token", Value::Text(generate_inbound_webhook_token()))];
        repo_iw::update_inbound_webhook(c, id, workspace_id, &set)?.ok_or_else(not_found)
    })
    .await
}

/// `deleteMany`: the removed ids (other workspaces' ids are ignored).
pub async fn delete_many(db: &Db, workspace_id: i64, ids: Vec<i64>) -> Result<Vec<i64>> {
    tx::write(db, move |c| {
        Ok(repo_iw::delete_inbound_webhooks(c, workspace_id, &ids)?)
    })
    .await
}

/// `delete`: 404 when nothing was removed.
pub async fn delete(db: &Db, workspace_id: i64, id: i64) -> Result<()> {
    let deleted = delete_many(db, workspace_id, vec![id]).await?;
    if deleted.is_empty() {
        return Err(not_found());
    }
    Ok(())
}

/// `recordRequest`: `None` = a duplicate `Idempotency-Key` for this webhook.
pub async fn record_request(
    db: &Db,
    webhook: &InboundWebhookIntake,
    payload: Json,
    idempotency_key: Option<String>,
    client_ip: Option<String>,
    user_agent: Option<String>,
) -> Result<Option<InboundWebhookRequest>> {
    let (workspace_id, webhook_id) = (webhook.workspace_id, webhook.id);
    let text = |v: Option<String>| v.map_or(Value::Null, Value::Text);
    tx::write(db, move |c| {
        let values: Patch = vec![
            ("workspaceId", Value::Integer(workspace_id)),
            ("inboundWebhookId", Value::Integer(webhook_id)),
            ("idempotencyKey", text(idempotency_key)),
            ("clientIp", text(client_ip)),
            ("userAgent", text(user_agent)),
            ("status", Value::Text("received".into())),
            ("attempts", Value::Integer(0)),
            ("payload", json_value(&payload)),
        ];
        Ok(repo_iw::insert_inbound_webhook_request(c, &values)?)
    })
    .await
}

/// `listRequests`: `(rows, pageCount)`, newest first.
pub async fn list_requests(
    db: &Db,
    workspace_id: i64,
    inbound_webhook_id: i64,
    status: Option<String>,
    page: i64,
    per_page: i64,
) -> Result<(Vec<InboundWebhookRequestListRow>, i64)> {
    let per_page = per_page.max(1);
    let input = ListInboundWebhookRequestsInput {
        workspace_id,
        inbound_webhook_id,
        status,
        limit: per_page,
        offset: (page.max(1) - 1) * per_page,
    };
    let out = tx::read(db, move |c| {
        Ok(repo_iw::list_inbound_webhook_requests_paginated(c, &input)?)
    })
    .await?;
    Ok((out.rows, (out.total + per_page - 1) / per_page))
}

/// `findRequestOrFail`.
pub async fn find_request_or_fail(
    db: &Db,
    workspace_id: i64,
    inbound_webhook_id: i64,
    id: i64,
) -> Result<InboundWebhookRequest> {
    tx::read(db, move |c| {
        repo_iw::find_inbound_webhook_request(c, id, workspace_id, Some(inbound_webhook_id))?
            .ok_or_else(|| BusinessError::not_found(REQUEST_NOT_FOUND))
    })
    .await
}

/// `resolveWorkspaceFreezeReasonById` without the owner check (community):
/// `true` when the workspace is missing or scheduled for deletion.
///
/// ponytail: the cloud-only `ownerBlocked` (403) answer is not ported.
pub async fn workspace_unavailable(db: &Db, workspace_id: i64) -> Result<bool> {
    tx::read(db, move |c| {
        let row: Option<Option<i64>> = c
            .query_row(
                r#"SELECT "scheduledDeletionAt" FROM "Workspace" WHERE "id" = ?1"#,
                [workspace_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(chatbotx_db::repo::Error::from)?;
        Ok(!matches!(row, Some(None)))
    })
    .await
}
