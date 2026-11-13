//! Port of `repositories/inbound-webhook/repository.ts` (P3.2b).
//!
//! Worker writes are fenced by the claim (`status = 'processing'` and the claim token).
//! "Now" is a parameter (`now`, unix ms).

use rusqlite::Connection;

use super::Result;
use super::chunked_purge::ChunkedPurgeStopReason;
use super::models::{InboundWebhook, InboundWebhookRequest};
use super::patch::Patch;
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

/// `insertInboundWebhook` (`id` / timestamps are filled when the patch has none).
pub fn insert_inbound_webhook(_conn: &Connection, _values: &Patch) -> Result<InboundWebhook> {
    todo!("P3.2b")
}

/// `updateInboundWebhook` (`set` keys: `name`, `active`, `token`, `inboxResolution`,
/// `identifiers`, `fieldMappings`, `actions`, `sampleJson`; `updatedAt` is set too).
pub fn update_inbound_webhook(
    _conn: &Connection,
    _id: i64,
    _workspace_id: i64,
    _set: &Patch,
) -> Result<Option<InboundWebhook>> {
    todo!("P3.2b")
}

/// `findInboundWebhookById`.
pub fn find_inbound_webhook_by_id(
    _conn: &Connection,
    _id: i64,
    _workspace_id: i64,
) -> Result<Option<InboundWebhook>> {
    todo!("P3.2b")
}

/// `findInboundWebhookIntakeByToken`: unscoped on purpose (the token is the capability).
pub fn find_inbound_webhook_intake_by_token(
    _conn: &Connection,
    _token: &str,
) -> Result<Option<InboundWebhookIntake>> {
    todo!("P3.2b")
}

/// `countInboundWebhooks`.
pub fn count_inbound_webhooks(_conn: &Connection, _workspace_id: i64) -> Result<i64> {
    todo!("P3.2b")
}

/// `listInboundWebhooksPaginated`: newest first.
pub fn list_inbound_webhooks_paginated(
    _conn: &Connection,
    _workspace_id: i64,
    _limit: i64,
    _offset: i64,
) -> Result<Paginated<InboundWebhookListRow>> {
    todo!("P3.2b")
}

/// `deleteInboundWebhooks`: the ids it removed (ids of another workspace are ignored).
pub fn delete_inbound_webhooks(
    _conn: &Connection,
    _workspace_id: i64,
    _ids: &[i64],
) -> Result<Vec<i64>> {
    todo!("P3.2b")
}

/// `insertInboundWebhookRequest`: `None` when the idempotency key is already recorded
/// for the webhook; otherwise the row, and the webhook's `requestCount` moves by one
/// (`updatedAt` unchanged). Run it inside one write job.
pub fn insert_inbound_webhook_request(
    _conn: &Connection,
    _values: &Patch,
) -> Result<Option<InboundWebhookRequest>> {
    todo!("P3.2b")
}

/// `claimInboundWebhookRequest`: a `received` row, or a `processing` row whose lease
/// expired (`claimedAt < lease_expired_before`), in one UPDATE.
pub fn claim_inbound_webhook_request(
    _conn: &Connection,
    _id: i64,
    _claim_token: &str,
    _lease_expired_before: i64,
    _now: i64,
) -> Result<Option<InboundWebhookRequest>> {
    todo!("P3.2b")
}

/// `checkpointInboundWebhookRequest`: records progress and renews the lease.
pub fn checkpoint_inbound_webhook_request(
    _conn: &Connection,
    _id: i64,
    _claim_token: &str,
    _progress: &InboundWebhookRequestProgress,
    _now: i64,
) -> Result<bool> {
    todo!("P3.2b")
}

/// `releaseInboundWebhookRequest`: `processing` -> `received`.
pub fn release_inbound_webhook_request(
    _conn: &Connection,
    _id: i64,
    _claim_token: &str,
) -> Result<bool> {
    todo!("P3.2b")
}

/// `completeInboundWebhookRequest`: the terminal status and `processedAt`.
pub fn complete_inbound_webhook_request(
    _conn: &Connection,
    _id: i64,
    _claim_token: &str,
    _completion: &InboundWebhookRequestCompletion,
    _now: i64,
) -> Result<bool> {
    todo!("P3.2b")
}

/// `findInboundWebhookRequest`.
pub fn find_inbound_webhook_request(
    _conn: &Connection,
    _id: i64,
    _workspace_id: i64,
    _inbound_webhook_id: Option<i64>,
) -> Result<Option<InboundWebhookRequest>> {
    todo!("P3.2b")
}

/// `listInboundWebhookRequestsPaginated`: newest first.
pub fn list_inbound_webhook_requests_paginated(
    _conn: &Connection,
    _input: &ListInboundWebhookRequestsInput,
) -> Result<Paginated<InboundWebhookRequestListRow>> {
    todo!("P3.2b")
}

/// `listStaleInboundWebhookRequests`: unfinished requests created after
/// `created_after`, oldest first.
pub fn list_stale_inbound_webhook_requests(
    _conn: &Connection,
    _received_before: i64,
    _lease_expired_before: i64,
    _created_after: i64,
    _limit: i64,
) -> Result<Vec<StaleInboundWebhookRequestRow>> {
    todo!("P3.2b")
}

/// `failAbandonedInboundWebhookRequests`: closes unfinished requests created before
/// `created_before` as `failed` / `abandoned`; returns how many.
pub fn fail_abandoned_inbound_webhook_requests(
    _conn: &Connection,
    _created_before: i64,
    _received_before: i64,
    _lease_expired_before: i64,
    _now: i64,
) -> Result<i64> {
    todo!("P3.2b")
}

/// `purgeInboundWebhookRequests`: chunked delete by age alone.
pub async fn purge_inbound_webhook_requests(
    _db: &Db,
    _options: PurgeInboundWebhookRequestsOptions,
    _now: i64,
) -> Result<(i64, ChunkedPurgeStopReason)> {
    todo!("P3.2b")
}
