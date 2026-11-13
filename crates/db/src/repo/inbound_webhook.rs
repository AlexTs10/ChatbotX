//! Port of `repositories/inbound-webhook/repository.ts` (P3.2b).
//!
//! Worker writes are fenced by the claim (`status = 'processing'` and the claim token).
//! "Now" is a parameter (`now`, unix ms).

use rusqlite::Connection;
use rusqlite::types::Value;

use super::chunked_purge::{ChunkedPurgeOptions, ChunkedPurgeStopReason, chunked_purge};
use super::models::{InboundWebhook, InboundWebhookRequest};
use super::patch::{Patch, check, insert_clause, json, set_clause};
use super::sql::{Sql, opt_int, opt_text};
use super::{Error, Model, Result, find_one, id_list, new_id, now_ms};
use crate::Db;

/// `INBOUND_WEBHOOK_REQUEST_ERROR_CODE.abandoned`.
pub const ERROR_CODE_ABANDONED: &str = "abandoned";

/// `InboundWebhookIntake`: what the delivery hot path reads (never the config).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboundWebhookIntake {
    pub id: i64,
    pub workspace_id: i64,
    pub active: bool,
    pub is_configured: bool,
}

/// `InboundWebhookListRow`: never the token, the config or the sample.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboundWebhookListRow {
    pub id: i64,
    pub name: String,
    pub active: bool,
    pub created_at: i64,
    pub updated_at: i64,
    pub request_count: i64,
}

/// `InboundWebhookRequestListRow`: never the payload, field errors or action results.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboundWebhookRequestListRow {
    pub id: i64,
    pub status: String,
    pub skip_reason: Option<String>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub inbox_id: Option<i64>,
    pub contact_id: Option<i64>,
    pub attempts: i64,
    pub created_at: i64,
    pub processed_at: Option<i64>,
}

/// `{ rows, total }` of the paginated lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paginated<T> {
    pub rows: Vec<T>,
    pub total: i64,
}

/// `StaleInboundWebhookRequestRow`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaleInboundWebhookRequestRow {
    pub id: i64,
    pub workspace_id: i64,
    pub inbound_webhook_id: i64,
}

/// `InboundWebhookRequestProgress`: `None` = key absent (not written), `Some(None)` =
/// written as NULL.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct InboundWebhookRequestProgress {
    pub inbox_id: Option<Option<i64>>,
    pub contact_id: Option<Option<i64>>,
    pub contact_inbox_id: Option<Option<i64>>,
    pub field_errors: Option<Option<serde_json::Value>>,
    pub action_results: Option<Option<serde_json::Value>>,
}

/// `InboundWebhookRequestCompletion` (`status` is a terminal status).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct InboundWebhookRequestCompletion {
    pub progress: InboundWebhookRequestProgress,
    pub status: String,
    pub skip_reason: Option<Option<String>>,
    pub error_code: Option<Option<String>>,
    pub error_message: Option<Option<String>>,
}

/// Input of `listInboundWebhookRequestsPaginated`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListInboundWebhookRequestsInput {
    pub workspace_id: i64,
    pub inbound_webhook_id: i64,
    pub status: Option<String>,
    pub limit: i64,
    pub offset: i64,
}

/// `PurgeInboundWebhookRequestsOptions`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PurgeInboundWebhookRequestsOptions {
    pub retention_days: i64,
    pub chunk_size: i64,
    pub inter_chunk_delay_ms: u64,
    pub max_chunks: i64,
    pub max_run_duration_ms: Option<i64>,
}

/// Fills `id` / `createdAt` / `updatedAt` when the patch has none.
fn with_defaults(values: &Patch) -> Patch {
    let now = now_ms();
    let mut all: Patch = Vec::new();
    for (col, v) in [
        ("id", Value::Integer(new_id())),
        ("createdAt", Value::Integer(now)),
        ("updatedAt", Value::Integer(now)),
    ] {
        if !values.iter().any(|(c, _)| *c == col) {
            all.push((col, v));
        }
    }
    all.extend(values.iter().cloned());
    all
}

/// The keys of `UpdateInboundWebhook`.
const UPDATE_KEYS: &[&str] = &[
    "name",
    "active",
    "token",
    "inboxResolution",
    "identifiers",
    "fieldMappings",
    "actions",
    "sampleJson",
];

const REQUEST_LIST_COLUMNS: &str = "\"id\", \"status\", \"skipReason\", \"errorCode\", \"errorMessage\", \
     \"inboxId\", \"contactId\", \"attempts\", \"createdAt\", \"processedAt\"";

/// `ownedByClaim`: only the current owner of the claim may write.
fn owned_by_claim(q: &mut Sql, id: i64, claim_token: &str) {
    q.push(" WHERE \"id\" = ");
    q.push_bind(id);
    q.push(" AND \"status\" = 'processing' AND \"claimToken\" = ");
    q.push_bind(claim_token.to_owned());
}

/// `unfinishedRequest`: `received` too long, or `processing` past its lease.
fn unfinished_request(q: &mut Sql, received_before: i64, lease_expired_before: i64) {
    q.push(" AND ((\"status\" = 'received' AND \"createdAt\" < ");
    q.push_bind(received_before);
    q.push(") OR (\"status\" = 'processing' AND \"claimedAt\" < ");
    q.push_bind(lease_expired_before);
    q.push("))");
}

fn json_value(v: Option<&serde_json::Value>) -> Value {
    v.map_or(Value::Null, json)
}

/// The keys of `progress` that are present, as SET pairs.
fn progress_patch(progress: &InboundWebhookRequestProgress) -> Patch {
    let mut out: Patch = Vec::new();
    for (col, v) in [
        ("inboxId", progress.inbox_id),
        ("contactId", progress.contact_id),
        ("contactInboxId", progress.contact_inbox_id),
    ] {
        if let Some(v) = v {
            out.push((col, opt_int(v)));
        }
    }
    for (col, v) in [
        ("fieldErrors", &progress.field_errors),
        ("actionResults", &progress.action_results),
    ] {
        if let Some(v) = v {
            out.push((col, json_value(v.as_ref())));
        }
    }
    out
}

/// Runs a fenced UPDATE with `set`; whether the row was written.
fn fenced_update(conn: &Connection, id: i64, claim_token: &str, set: &Patch) -> Result<bool> {
    let mut q = Sql::raw("UPDATE \"InboundWebhookRequest\" SET ");
    q.append(set_clause(InboundWebhookRequest::COLUMNS, set)?);
    owned_by_claim(&mut q, id, claim_token);
    Ok(q.execute(conn)? > 0)
}

fn first_i64(conn: &Connection, q: &Sql) -> Result<i64> {
    Ok(q.query_map(conn, |r| r.get::<_, i64>(0))?
        .into_iter()
        .next()
        .unwrap_or(0))
}

/// `insertInboundWebhook` (`id` / timestamps are filled when the patch has none).
pub fn insert_inbound_webhook(conn: &Connection, values: &Patch) -> Result<InboundWebhook> {
    let mut q = Sql::raw("INSERT INTO \"InboundWebhook\" ");
    q.append(insert_clause(
        InboundWebhook::COLUMNS,
        &with_defaults(values),
    )?);
    q.push(&format!(" RETURNING {}", InboundWebhook::columns_sql(None)));
    q.model(conn)?
        .ok_or_else(|| Error::msg("Failed to insert inbound webhook"))
}

/// `updateInboundWebhook` (`set` keys: `name`, `active`, `token`, `inboxResolution`,
/// `identifiers`, `fieldMappings`, `actions`, `sampleJson`; `updatedAt` is set too).
pub fn update_inbound_webhook(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    set: &Patch,
) -> Result<Option<InboundWebhook>> {
    check(UPDATE_KEYS, set)?;
    let mut all = set.clone();
    all.push(("updatedAt", Value::Integer(now_ms())));
    let mut q = Sql::raw("UPDATE \"InboundWebhook\" SET ");
    q.append(set_clause(InboundWebhook::COLUMNS, &all)?);
    q.push(" WHERE \"id\" = ");
    q.push_bind(id);
    q.push(" AND \"workspaceId\" = ");
    q.push_bind(workspace_id);
    q.push(&format!(" RETURNING {}", InboundWebhook::columns_sql(None)));
    q.model(conn)
}

/// `findInboundWebhookById`.
pub fn find_inbound_webhook_by_id(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
) -> Result<Option<InboundWebhook>> {
    find_one(
        conn,
        "\"id\" = ?1 AND \"workspaceId\" = ?2",
        &[&id, &workspace_id],
    )
}

/// `findInboundWebhookIntakeByToken`: unscoped on purpose (the token is the capability).
pub fn find_inbound_webhook_intake_by_token(
    conn: &Connection,
    token: &str,
) -> Result<Option<InboundWebhookIntake>> {
    Ok(Sql::new(
        "SELECT \"id\", \"workspaceId\", \"active\", \"inboxResolution\" IS NOT NULL \
         FROM \"InboundWebhook\" WHERE \"token\" = ? LIMIT 1",
        vec![Value::Text(token.to_owned())],
    )
    .query_map(conn, |r| {
        Ok(InboundWebhookIntake {
            id: r.get(0)?,
            workspace_id: r.get(1)?,
            active: r.get(2)?,
            is_configured: r.get(3)?,
        })
    })?
    .into_iter()
    .next())
}

/// `countInboundWebhooks`.
pub fn count_inbound_webhooks(conn: &Connection, workspace_id: i64) -> Result<i64> {
    first_i64(
        conn,
        &Sql::bind(
            "SELECT count(*) FROM \"InboundWebhook\" WHERE \"workspaceId\" = ",
            workspace_id,
        ),
    )
}

/// `listInboundWebhooksPaginated`: newest first.
pub fn list_inbound_webhooks_paginated(
    conn: &Connection,
    workspace_id: i64,
    limit: i64,
    offset: i64,
) -> Result<Paginated<InboundWebhookListRow>> {
    let rows = Sql::new(
        "SELECT \"id\", \"name\", \"active\", \"createdAt\", \"updatedAt\", \"requestCount\" \
         FROM \"InboundWebhook\" WHERE \"workspaceId\" = ? \
         ORDER BY \"createdAt\" DESC, \"id\" DESC LIMIT ? OFFSET ?",
        vec![
            Value::Integer(workspace_id),
            Value::Integer(limit),
            Value::Integer(offset),
        ],
    )
    .query_map(conn, |r| {
        Ok(InboundWebhookListRow {
            id: r.get(0)?,
            name: r.get(1)?,
            active: r.get(2)?,
            created_at: r.get(3)?,
            updated_at: r.get(4)?,
            request_count: r.get(5)?,
        })
    })?;
    Ok(Paginated {
        rows,
        total: count_inbound_webhooks(conn, workspace_id)?,
    })
}

/// `deleteInboundWebhooks`: the ids it removed (ids of another workspace are ignored).
pub fn delete_inbound_webhooks(
    conn: &Connection,
    workspace_id: i64,
    ids: &[i64],
) -> Result<Vec<i64>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    Sql::new(
        "DELETE FROM \"InboundWebhook\" WHERE \"workspaceId\" = ? \
         AND \"id\" IN (SELECT value FROM json_each(?)) RETURNING \"id\"",
        vec![Value::Integer(workspace_id), Value::Text(id_list(ids))],
    )
    .query_map(conn, |r| r.get(0))
}

/// `insertInboundWebhookRequest`: `None` when the idempotency key is already recorded
/// for the webhook; otherwise the row, and the webhook's `requestCount` moves by one
/// (`updatedAt` unchanged). Run it inside one write job.
pub fn insert_inbound_webhook_request(
    conn: &Connection,
    values: &Patch,
) -> Result<Option<InboundWebhookRequest>> {
    let mut q = Sql::raw("INSERT INTO \"InboundWebhookRequest\" ");
    q.append(insert_clause(
        InboundWebhookRequest::COLUMNS,
        &with_defaults(values),
    )?);
    q.push(&format!(
        " ON CONFLICT DO NOTHING RETURNING {}",
        InboundWebhookRequest::columns_sql(None)
    ));
    let Some(row) = q.model::<InboundWebhookRequest>(conn)? else {
        return Ok(None);
    };
    Sql::bind(
        "UPDATE \"InboundWebhook\" SET \"requestCount\" = \"requestCount\" + 1 WHERE \"id\" = ",
        row.inbound_webhook_id,
    )
    .execute(conn)?;
    Ok(Some(row))
}

/// `claimInboundWebhookRequest`: a `received` row, or a `processing` row whose lease
/// expired (`claimedAt < lease_expired_before`), in one UPDATE.
pub fn claim_inbound_webhook_request(
    conn: &Connection,
    id: i64,
    claim_token: &str,
    lease_expired_before: i64,
    now: i64,
) -> Result<Option<InboundWebhookRequest>> {
    Sql::new(
        format!(
            "UPDATE \"InboundWebhookRequest\" SET \"status\" = 'processing', \"claimToken\" = ?, \
             \"claimedAt\" = ?, \"attempts\" = \"attempts\" + 1, \"updatedAt\" = ? \
             WHERE \"id\" = ? AND (\"status\" = 'received' OR (\"status\" = 'processing' AND \"claimedAt\" < ?)) \
             RETURNING {}",
            InboundWebhookRequest::columns_sql(None)
        ),
        vec![
            Value::Text(claim_token.to_owned()),
            Value::Integer(now),
            Value::Integer(now),
            Value::Integer(id),
            Value::Integer(lease_expired_before),
        ],
    )
    .model(conn)
}

/// `checkpointInboundWebhookRequest`: records progress and renews the lease.
pub fn checkpoint_inbound_webhook_request(
    conn: &Connection,
    id: i64,
    claim_token: &str,
    progress: &InboundWebhookRequestProgress,
    now: i64,
) -> Result<bool> {
    let mut set = progress_patch(progress);
    set.push(("claimedAt", Value::Integer(now)));
    set.push(("updatedAt", Value::Integer(now)));
    fenced_update(conn, id, claim_token, &set)
}

/// `releaseInboundWebhookRequest`: `processing` -> `received`.
pub fn release_inbound_webhook_request(
    conn: &Connection,
    id: i64,
    claim_token: &str,
) -> Result<bool> {
    let set: Patch = vec![
        ("status", Value::Text("received".to_owned())),
        ("claimToken", Value::Null),
        ("claimedAt", Value::Null),
        ("updatedAt", Value::Integer(now_ms())),
    ];
    fenced_update(conn, id, claim_token, &set)
}

/// `completeInboundWebhookRequest`: the terminal status and `processedAt`.
pub fn complete_inbound_webhook_request(
    conn: &Connection,
    id: i64,
    claim_token: &str,
    completion: &InboundWebhookRequestCompletion,
    now: i64,
) -> Result<bool> {
    let mut set = progress_patch(&completion.progress);
    set.push(("status", Value::Text(completion.status.clone())));
    for (col, v) in [
        ("skipReason", &completion.skip_reason),
        ("errorCode", &completion.error_code),
        ("errorMessage", &completion.error_message),
    ] {
        if let Some(v) = v {
            set.push((col, opt_text(v.as_deref())));
        }
    }
    set.push(("processedAt", Value::Integer(now)));
    set.push(("updatedAt", Value::Integer(now)));
    fenced_update(conn, id, claim_token, &set)
}

/// `findInboundWebhookRequest`.
pub fn find_inbound_webhook_request(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    inbound_webhook_id: Option<i64>,
) -> Result<Option<InboundWebhookRequest>> {
    let mut q = Sql::raw(format!(
        "{} WHERE \"id\" = ",
        InboundWebhookRequest::select_sql()
    ));
    q.push_bind(id);
    q.push(" AND \"workspaceId\" = ");
    q.push_bind(workspace_id);
    if let Some(hook) = inbound_webhook_id {
        q.push(" AND \"inboundWebhookId\" = ");
        q.push_bind(hook);
    }
    q.push(" LIMIT 1");
    q.model(conn)
}

/// `listInboundWebhookRequestsPaginated`: newest first.
pub fn list_inbound_webhook_requests_paginated(
    conn: &Connection,
    input: &ListInboundWebhookRequestsInput,
) -> Result<Paginated<InboundWebhookRequestListRow>> {
    let filter = || {
        let mut w = Sql::bind(" WHERE \"workspaceId\" = ", input.workspace_id);
        w.push(" AND \"inboundWebhookId\" = ");
        w.push_bind(input.inbound_webhook_id);
        if let Some(status) = &input.status {
            w.push(" AND \"status\" = ");
            w.push_bind(status.clone());
        }
        w
    };
    let mut q = Sql::raw(format!(
        "SELECT {REQUEST_LIST_COLUMNS} FROM \"InboundWebhookRequest\""
    ));
    q.append(filter());
    q.push(" ORDER BY \"createdAt\" DESC, \"id\" DESC LIMIT ");
    q.push_bind(input.limit);
    q.push(" OFFSET ");
    q.push_bind(input.offset);
    let rows = q.query_map(conn, |r| {
        Ok(InboundWebhookRequestListRow {
            id: r.get(0)?,
            status: r.get(1)?,
            skip_reason: r.get(2)?,
            error_code: r.get(3)?,
            error_message: r.get(4)?,
            inbox_id: r.get(5)?,
            contact_id: r.get(6)?,
            attempts: r.get(7)?,
            created_at: r.get(8)?,
            processed_at: r.get(9)?,
        })
    })?;
    let mut total = Sql::raw("SELECT count(*) FROM \"InboundWebhookRequest\"");
    total.append(filter());
    Ok(Paginated {
        rows,
        total: first_i64(conn, &total)?,
    })
}

/// `listStaleInboundWebhookRequests`: unfinished requests created after
/// `created_after`, oldest first.
pub fn list_stale_inbound_webhook_requests(
    conn: &Connection,
    received_before: i64,
    lease_expired_before: i64,
    created_after: i64,
    limit: i64,
) -> Result<Vec<StaleInboundWebhookRequestRow>> {
    let mut q = Sql::bind(
        "SELECT \"id\", \"workspaceId\", \"inboundWebhookId\" FROM \"InboundWebhookRequest\" \
         WHERE \"createdAt\" > ",
        created_after,
    );
    unfinished_request(&mut q, received_before, lease_expired_before);
    q.push(" ORDER BY \"createdAt\" ASC, \"id\" ASC LIMIT ");
    q.push_bind(limit);
    q.query_map(conn, |r| {
        Ok(StaleInboundWebhookRequestRow {
            id: r.get(0)?,
            workspace_id: r.get(1)?,
            inbound_webhook_id: r.get(2)?,
        })
    })
}

/// `failAbandonedInboundWebhookRequests`: closes unfinished requests created before
/// `created_before` as `failed` / `abandoned`; returns how many.
pub fn fail_abandoned_inbound_webhook_requests(
    conn: &Connection,
    created_before: i64,
    received_before: i64,
    lease_expired_before: i64,
    now: i64,
) -> Result<i64> {
    let mut q = Sql::new(
        "UPDATE \"InboundWebhookRequest\" SET \"status\" = 'failed', \"errorCode\" = ?, \
         \"claimToken\" = NULL, \"processedAt\" = ?, \"updatedAt\" = ? WHERE \"createdAt\" < ?",
        vec![
            Value::Text(ERROR_CODE_ABANDONED.to_owned()),
            Value::Integer(now),
            Value::Integer(now),
            Value::Integer(created_before),
        ],
    );
    unfinished_request(&mut q, received_before, lease_expired_before);
    Ok(i64::try_from(q.execute(conn)?).unwrap_or(i64::MAX))
}

/// `purgeInboundWebhookRequests`: chunked delete by age alone.
pub async fn purge_inbound_webhook_requests(
    db: &Db,
    options: PurgeInboundWebhookRequestsOptions,
    now: i64,
) -> Result<(i64, ChunkedPurgeStopReason)> {
    let cutoff = now - options.retention_days * 86_400_000;
    chunked_purge(
        db,
        &ChunkedPurgeOptions {
            table: "InboundWebhookRequest".into(),
            filter: Sql::bind("\"createdAt\" < ", cutoff),
            order_by: "createdAt".into(),
            chunk_size: options.chunk_size,
            inter_chunk_delay_ms: options.inter_chunk_delay_ms,
            max_chunks: options.max_chunks,
            max_run_duration_ms: options.max_run_duration_ms,
        },
    )
    .await
}
