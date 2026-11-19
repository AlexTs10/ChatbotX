//! `packages/business/src/inbound-webhook/service.ts`: CRUD, settings,
//! token, request log and intake writes.
//!
//! ponytail: no token cache (the intake reads one indexed row per delivery)
//! and no config lock: a settings change reads, checks and writes in one
//! SQLite write job, which is already serial.
//! `previewMapping` and the payload apply: `inbound_webhook_apply`.

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
fn assert_config_valid(
    c: &Connection,
    workspace_id: i64,
    resolution: Option<&Json>,
    identifiers: &Json,
    field_mappings: &Json,
) -> Result<()> {
    let empty = |v: &Json| v.as_array().is_none_or(Vec::is_empty);
    let resolution = resolution.filter(|v| !v.is_null());
    let issue = if resolution.is_none() {
        Some(("inboxResolution", "inboxRequired"))
    } else if empty(identifiers) {
        Some(("identifiers", "identifierRequired"))
    } else if empty(field_mappings) {
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

/// The saved config (`inboundWebhookConfigRequest`, parsed and canonical).
#[derive(Debug, Clone)]
pub struct Config {
    pub inbox_resolution: Option<Json>,
    pub identifiers: Json,
    pub field_mappings: Json,
    pub actions: Json,
    pub sample_json: Option<Json>,
}

/// `assertReferencesExist`: mapped custom fields and the actions' flows,
/// tags and fields must belong to the workspace.
fn assert_references_exist(c: &Connection, workspace_id: i64, config: &Config) -> Result<()> {
    let unknown_target = || {
        BusinessError::validation(
            "fieldMappings",
            "validation.inboundWebhook.unknownFieldTarget",
        )
    };
    let mut ids: Vec<i64> = Vec::new();
    for mapping in config.field_mappings.as_array().into_iter().flatten() {
        let target = mapping
            .get("target")
            .and_then(Json::as_str)
            .unwrap_or_default();
        if target == "gender" || crate::questionnaire::is_rich_system_contact_field(target) {
            continue;
        }
        if target.is_empty() || !target.bytes().all(|b| b.is_ascii_digit()) {
            return Err(unknown_target());
        }
        ids.push(target.parse().map_err(|_| unknown_target())?);
    }
    ids.sort_unstable();
    ids.dedup();
    if !ids.is_empty() {
        let found: i64 = c
            .query_row(
                r#"SELECT count(*) FROM "CustomField" WHERE "workspaceId" = ?1
                   AND "id" IN (SELECT value FROM json_each(?2))"#,
                chatbotx_db::rusqlite::params![workspace_id, chatbotx_db::repo::id_list(&ids)],
                |r| r.get(0),
            )
            .map_err(chatbotx_db::repo::Error::from)?;
        if usize::try_from(found).ok() != Some(ids.len()) {
            return Err(unknown_target());
        }
    }
    let actions = config.actions.as_array().cloned().unwrap_or_default();
    if !crate::automation_action::find_invalid_action_references(
        c,
        workspace_id,
        &actions,
        None,
        false,
    )?
    .is_empty()
    {
        return Err(BusinessError::validation(
            "actions",
            "validation.inboundWebhook.actionReferenceNotFound",
        ));
    }
    Ok(())
}

/// `update`: 404 first, then the config checks, then the row is replaced.
pub async fn update(db: &Db, workspace_id: i64, id: i64, config: Config) -> Result<InboundWebhook> {
    tx::write(db, move |c| {
        find_in(c, workspace_id, id)?;
        assert_config_valid(
            c,
            workspace_id,
            config.inbox_resolution.as_ref(),
            &config.identifiers,
            &config.field_mappings,
        )?;
        assert_references_exist(c, workspace_id, &config)?;
        let opt = |v: &Option<Json>| v.as_ref().map_or(Value::Null, json_value);
        let set: Patch = vec![
            ("inboxResolution", opt(&config.inbox_resolution)),
            ("identifiers", json_value(&config.identifiers)),
            ("fieldMappings", json_value(&config.field_mappings)),
            ("actions", json_value(&config.actions)),
            ("sampleJson", opt(&config.sample_json)),
        ];
        repo_iw::update_inbound_webhook(c, id, workspace_id, &set)?.ok_or_else(not_found)
    })
    .await
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
            assert_config_valid(
                c,
                workspace_id,
                existing.inbox_resolution.as_ref().map(|j| &j.0),
                &existing.identifiers.0,
                &existing.field_mappings.0,
            )?;
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

/// `CLAIM_LEASE_MS`.
pub const CLAIM_LEASE_MS: i64 = 15 * 60 * 1000;
/// `UNCLAIMED_REQUEST_GRACE_MS`.
const UNCLAIMED_REQUEST_GRACE_MS: i64 = 2 * 60 * 1000;
/// `REDISPATCH_MAX_AGE_MS`.
const REDISPATCH_MAX_AGE_MS: i64 = 24 * 60 * 60 * 1000;

/// `claimRequest`: the request and a fresh claim token, or `None` when
/// another worker owns it (or it is finished).
pub async fn claim_request(db: &Db, id: i64) -> Result<Option<(InboundWebhookRequest, String)>> {
    let token = chatbotx_db::repo::random_uuid();
    tx::write(db, move |c| {
        let now = chatbotx_db::repo::now_ms();
        let row = repo_iw::claim_inbound_webhook_request(c, id, &token, now - CLAIM_LEASE_MS, now)?;
        Ok(row.map(|r| (r, token)))
    })
    .await
}

/// `checkpointRequest`: whether the claim is still ours.
pub async fn checkpoint_request(
    db: &Db,
    id: i64,
    claim_token: String,
    progress: repo_iw::InboundWebhookRequestProgress,
) -> Result<bool> {
    tx::write(db, move |c| {
        Ok(repo_iw::checkpoint_inbound_webhook_request(
            c,
            id,
            &claim_token,
            &progress,
            chatbotx_db::repo::now_ms(),
        )?)
    })
    .await
}

/// `completeRequest`.
pub async fn complete_request(
    db: &Db,
    id: i64,
    claim_token: String,
    completion: repo_iw::InboundWebhookRequestCompletion,
) -> Result<bool> {
    tx::write(db, move |c| {
        Ok(repo_iw::complete_inbound_webhook_request(
            c,
            id,
            &claim_token,
            &completion,
            chatbotx_db::repo::now_ms(),
        )?)
    })
    .await
}

/// `releaseRequest`: `processing` back to `received`.
pub async fn release_request(db: &Db, id: i64, claim_token: String) -> Result<bool> {
    tx::write(db, move |c| {
        Ok(repo_iw::release_inbound_webhook_request(
            c,
            id,
            &claim_token,
        )?)
    })
    .await
}

/// `listStaleRequests`: unfinished requests the re-dispatch cron re-enqueues.
pub async fn list_stale_requests(
    db: &Db,
    limit: i64,
) -> Result<Vec<repo_iw::StaleInboundWebhookRequestRow>> {
    tx::read(db, move |c| {
        let now = chatbotx_db::repo::now_ms();
        Ok(repo_iw::list_stale_inbound_webhook_requests(
            c,
            now - UNCLAIMED_REQUEST_GRACE_MS,
            now - CLAIM_LEASE_MS,
            now - REDISPATCH_MAX_AGE_MS,
            limit,
        )?)
    })
    .await
}

/// `abandonStaleRequests`: closes what re-dispatch stopped retrying.
pub async fn abandon_stale_requests(db: &Db) -> Result<i64> {
    tx::write(db, move |c| {
        let now = chatbotx_db::repo::now_ms();
        Ok(repo_iw::fail_abandoned_inbound_webhook_requests(
            c,
            now - REDISPATCH_MAX_AGE_MS,
            now - UNCLAIMED_REQUEST_GRACE_MS,
            now - CLAIM_LEASE_MS,
            now,
        )?)
    })
    .await
}
