//! Port of `repositories/coexist-sync-run/repository.ts` and `integration-access.ts` (P3.2b).
//!
//! `NOW()` is the wall clock in ms ([`now_ms`](super::now_ms)), bound once per statement.
//! `FOR UPDATE SKIP LOCKED` has no SQLite form: one writer runs at a time.

use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension, params};

use super::models::{
    CoexistSyncRun, IntegrationInstagram, IntegrationMessenger, IntegrationWhatsapp,
};
use super::patch::{self, Patch};
use super::sql::Sql;
use super::utils::{
    PaginationInput, SortDir, SortItem, get_pagination_with_defaults, order_by_sql, parse_order_by,
};
use super::{Error, Model, Result, new_id, now_ms, random_uuid, text_list};

/// `NOW() - INTERVAL '10 minutes'`: a running claim older than this is stale.
const STALE_CLAIM_MS: i64 = 10 * 60_000;
/// `NOW() - INTERVAL '10 seconds'`: an `init` run untouched this long is due.
const INIT_DUE_MS: i64 = 10_000;
/// `NOW() - INTERVAL '1 hour'`: a `running` run without a heartbeat this long is due.
const RUNNING_DUE_MS: i64 = 3_600_000;
/// `s."processedAt" IS NULL AND s."parseFailedAt" IS NULL`.
const PENDING_STAGING: &str = "s.\"processedAt\" IS NULL AND s.\"parseFailedAt\" IS NULL";
/// The `IN (...)` list of [`LIVE_RUN_STATUSES`].
const LIVE_STATUS_LIST: &str = "'init', 'running', 'waiting'";

/// `LIVE_RUN_STATUSES`.
pub const LIVE_RUN_STATUSES: &[&str] = &["init", "running", "waiting"];
/// `PULL_CLAIMABLE_STATUSES`.
pub const PULL_CLAIMABLE_STATUSES: &[&str] = &["init", "running"];
/// `CoexistRunProgressInput.fields` keys (the columns `update_progress` may set).
pub const PROGRESS_FIELDS: &[&str] = &[
    "status",
    "currentStep",
    "currentError",
    "currentScan",
    "totalScan",
    "importedContactCount",
    "importedMessageCount",
    "skippedCount",
    "failedCount",
    "lastSyncedAt",
    "lastHeartbeatAt",
    "finishedAt",
    "messengerSyncPhase",
    "currentPageNumber",
    "pendingPatches",
    "lastPhase",
    "lastChunkOrder",
    "syncProgress",
    "resumeCursor",
];

/// `CoexistRunCreateInput`.
#[derive(Debug, Clone, Copy)]
pub struct CreateRunInput<'a> {
    pub workspace_id: i64,
    pub integration_id: i64,
    pub channel: &'a str,
    pub trigger_source: &'a str,
}

/// `CreateContactScanRunInput`.
#[derive(Debug, Clone, Copy)]
pub struct CreateContactScanRunInput<'a> {
    pub workspace_id: i64,
    pub integration_id: i64,
    pub channel: &'a str,
    pub requested_by_user_id: Option<i64>,
    pub scan_from_at: i64,
    pub trigger_source: &'a str,
}

/// `CoexistRunWriteGuard`: pins `status` (and `claimToken` when one is given).
#[derive(Debug, Clone, Copy)]
pub struct WriteGuard<'a> {
    pub status: &'a str,
    pub claim_token: Option<&'a str>,
}

/// `PickedCoexistRun`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickedCoexistRun {
    pub id: i64,
    pub attempts: i64,
    pub channel: String,
    pub integration_id: i64,
    pub workspace_id: i64,
}

/// One row of `findStrandedCoexistWhatsappIntegrations`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StrandedIntegration {
    pub integration_id: i64,
    pub workspace_id: i64,
}

/// `IncrementProgressCounters` (`None` = no change).
#[derive(Debug, Clone, Copy, Default)]
pub struct Increments {
    pub current_scan: Option<i64>,
    pub imported_contact_count: Option<i64>,
    pub imported_message_count: Option<i64>,
    pub skipped_count: Option<i64>,
    pub failed_count: Option<i64>,
}

/// `findInitState` columns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitState {
    pub attempts: i64,
    pub current_error: Option<String>,
    pub messenger_sync_phase: String,
}

/// `findTerminalCounters` columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalCounters {
    pub imported_message_count: i64,
    pub skipped_count: i64,
    pub failed_count: i64,
}

/// `CoexistIntegrationRow`: the integration row tagged with its channel.
#[derive(Debug, Clone)]
pub enum CoexistIntegrationRow {
    Messenger(IntegrationMessenger),
    Instagram(IntegrationInstagram),
    Whatsapp(IntegrationWhatsapp),
}

/// Input of [`list_contact_scan_runs`].
#[derive(Debug, Clone, Default)]
pub struct ListContactScanRunsInput {
    pub workspace_id: i64,
    pub integration_id: Option<i64>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
    pub sort: Option<Vec<SortItem>>,
}

/// `{ data, pageCount }` of [`list_contact_scan_runs`].
#[derive(Debug, Clone)]
pub struct ContactScanRunPage {
    pub data: Vec<CoexistSyncRun>,
    pub page_count: i64,
}

impl CoexistIntegrationRow {
    /// `channel` of the tagged row.
    pub fn channel(&self) -> &'static str {
        match self {
            CoexistIntegrationRow::Messenger(_) => "messenger",
            CoexistIntegrationRow::Instagram(_) => "instagram",
            CoexistIntegrationRow::Whatsapp(_) => "whatsapp",
        }
    }

    /// `id` of the row.
    pub fn id(&self) -> i64 {
        match self {
            CoexistIntegrationRow::Messenger(r) => r.id,
            CoexistIntegrationRow::Instagram(r) => r.id,
            CoexistIntegrationRow::Whatsapp(r) => r.id,
        }
    }

    /// `coexistEnabled` of the row.
    pub fn coexist_enabled(&self) -> bool {
        match self {
            CoexistIntegrationRow::Messenger(r) => r.coexist_enabled,
            CoexistIntegrationRow::Instagram(r) => r.coexist_enabled,
            CoexistIntegrationRow::Whatsapp(r) => r.coexist_enabled,
        }
    }

    /// `coexistAiReadsSyncedHistory` of the row.
    pub fn coexist_ai_reads_synced_history(&self) -> bool {
        match self {
            CoexistIntegrationRow::Messenger(r) => r.coexist_ai_reads_synced_history,
            CoexistIntegrationRow::Instagram(r) => r.coexist_ai_reads_synced_history,
            CoexistIntegrationRow::Whatsapp(r) => r.coexist_ai_reads_synced_history,
        }
    }
}

/// `"id" = ? [AND "status" = ?] [AND "claimToken" = ?]` (`runWriteFilter`; an empty token
/// counts as absent, as the JS truthiness check).
fn write_filter(run_id: i64, guard: Option<&WriteGuard<'_>>) -> Sql {
    let mut q = Sql::bind("\"id\" = ", run_id);
    if let Some(guard) = guard {
        q.push(" AND \"status\" = ");
        q.push_bind(guard.status.to_owned());
        if let Some(token) = guard.claim_token.filter(|t| !t.is_empty()) {
            q.push(" AND \"claimToken\" = ");
            q.push_bind(token.to_owned());
        }
    }
    q
}

fn first(
    conn: &Connection,
    where_sql: &str,
    params: &[&dyn rusqlite::ToSql],
) -> Result<Option<CoexistSyncRun>> {
    super::find_one::<CoexistSyncRun>(conn, where_sql, params)
}

/// `createRun`: inserts an `init` coexist run; on a conflict, the live `init` run.
pub fn create_run(conn: &Connection, input: &CreateRunInput<'_>) -> Result<CoexistSyncRun> {
    let now = now_ms();
    let inserted = super::returning::<CoexistSyncRun>(
        conn,
        "INSERT INTO \"CoexistSyncRun\" (\"id\", \"createdAt\", \"updatedAt\", \"workspaceId\", \
         \"integrationId\", \"channel\", \"type\", \"status\", \"triggerSource\", \"pendingPatches\", \
         \"claimToken\") VALUES (?1, ?2, ?2, ?3, ?4, ?5, 'coexist', 'init', ?6, NULL, NULL) \
         ON CONFLICT DO NOTHING",
        &[
            &new_id(),
            &now,
            &input.workspace_id,
            &input.integration_id,
            &input.channel,
            &input.trigger_source,
        ],
    )?;
    if let Some(run) = inserted.into_iter().next() {
        return Ok(run);
    }
    find_active_init_run(conn, input.integration_id, input.channel)?
        .ok_or_else(|| Error::msg("CoexistSyncRun insert conflicted but no active init run found"))
}

/// `findActiveInitRun`.
pub fn find_active_init_run(
    conn: &Connection,
    integration_id: i64,
    channel: &str,
) -> Result<Option<CoexistSyncRun>> {
    first(
        conn,
        "\"integrationId\" = ?1 AND \"channel\" = ?2 AND \"status\" = 'init' AND \"type\" = 'coexist' \
         ORDER BY \"id\"",
        &[&integration_id, &channel],
    )
}

/// `findLatestRun`.
pub fn find_latest_run(
    conn: &Connection,
    workspace_id: i64,
    integration_id: i64,
    channel: &str,
) -> Result<Option<CoexistSyncRun>> {
    first(
        conn,
        "\"workspaceId\" = ?1 AND \"integrationId\" = ?2 AND \"channel\" = ?3 AND \"type\" = 'coexist' \
         ORDER BY \"createdAt\" DESC, \"id\" DESC",
        &[&workspace_id, &integration_id, &channel],
    )
}

/// `findLiveRun`.
pub fn find_live_run(
    conn: &Connection,
    integration_id: i64,
    channel: &str,
) -> Result<Option<CoexistSyncRun>> {
    first(
        conn,
        &format!(
            "\"integrationId\" = ?1 AND \"channel\" = ?2 AND \"status\" IN ({LIVE_STATUS_LIST}) \
             AND \"type\" = 'coexist' ORDER BY \"createdAt\" DESC, \"id\" DESC"
        ),
        &[&integration_id, &channel],
    )
}

/// `findRunById`.
pub fn find_run_by_id(conn: &Connection, run_id: i64) -> Result<Option<CoexistSyncRun>> {
    first(conn, "\"id\" = ?1", &[&run_id])
}

/// `findWorkspaceOwnerId`.
pub fn find_workspace_owner_id(conn: &Connection, workspace_id: i64) -> Result<Option<i64>> {
    Ok(conn
        .query_row(
            "SELECT \"ownerId\" FROM \"Workspace\" WHERE \"id\" = ?1",
            params![workspace_id],
            |r| r.get::<_, Option<i64>>(0),
        )
        .optional()?
        .flatten())
}

/// `claimRunWithNewToken` (`from_statuses` defaults to [`PULL_CLAIMABLE_STATUSES`]).
pub fn claim_run_with_new_token(
    conn: &Connection,
    run_id: i64,
    from_statuses: Option<&[&str]>,
) -> Result<Option<CoexistSyncRun>> {
    let now = now_ms();
    let statuses = text_list(from_statuses.unwrap_or(PULL_CLAIMABLE_STATUSES));
    let rows = super::returning::<CoexistSyncRun>(
        conn,
        "UPDATE \"CoexistSyncRun\" SET \"status\" = 'running', \"claimToken\" = ?2, \
         \"startedAt\" = COALESCE(\"startedAt\", ?3), \"lastHeartbeatAt\" = ?3, \"updatedAt\" = ?3 \
         WHERE \"id\" = ?1 AND \"status\" IN (SELECT value FROM json_each(?4)) \
         AND (\"status\" <> 'running' OR \"lastHeartbeatAt\" < ?5)",
        &[
            &run_id,
            &random_uuid(),
            &now,
            &statuses,
            &(now - STALE_CLAIM_MS),
        ],
    )?;
    Ok(rows.into_iter().next())
}

/// `markMaxAttemptsFailed`.
pub fn mark_max_attempts_failed(
    conn: &Connection,
    run_type: &str,
    max_attempts: i64,
) -> Result<()> {
    let now = now_ms();
    conn.execute(
        "UPDATE \"CoexistSyncRun\" SET \"status\" = 'failed', \
         \"currentError\" = 'Max scheduler retries exceeded', \"finishedAt\" = ?1, \
         \"resumeCursor\" = NULL, \"updatedAt\" = ?1 \
         WHERE \"type\" = ?2 AND \"attempts\" >= ?3 AND \"status\" IN ('init', 'running') \
         AND (\"lastHeartbeatAt\" IS NULL OR \"lastHeartbeatAt\" < ?4)",
        params![now, run_type, max_attempts, now - STALE_CLAIM_MS],
    )?;
    Ok(())
}

/// Maps `RETURNING "id", "attempts", "channel", "integrationId", "workspaceId", "createdAt"`
/// rows, ordered by `createdAt` (SQLite does not keep the sub-select order in `RETURNING`).
fn picked(
    conn: &Connection,
    sql: &str,
    params: &[&dyn rusqlite::ToSql],
) -> Result<Vec<PickedCoexistRun>> {
    let mut stmt = conn.prepare(sql)?;
    let mut rows = stmt
        .query_map(params, |r| {
            Ok((
                r.get::<_, i64>(5)?,
                PickedCoexistRun {
                    id: r.get(0)?,
                    attempts: r.get(1)?,
                    channel: r.get(2)?,
                    integration_id: r.get(3)?,
                    workspace_id: r.get(4)?,
                },
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.sort_by_key(|(created, run)| (*created, run.id));
    Ok(rows.into_iter().map(|(_, run)| run).collect())
}

const PICKED_COLUMNS: &str = "RETURNING \"id\", \"attempts\", \"channel\", \"integrationId\", \"workspaceId\", \"createdAt\"";

/// `pickDueRuns`.
pub fn pick_due_runs(
    conn: &Connection,
    run_type: &str,
    batch_size: i64,
    max_attempts: i64,
) -> Result<Vec<PickedCoexistRun>> {
    let now = now_ms();
    picked(
        conn,
        &format!(
            "UPDATE \"CoexistSyncRun\" SET \"attempts\" = \"attempts\" + 1, \"status\" = 'init', \
             \"updatedAt\" = ?1 WHERE \"id\" IN (SELECT \"id\" FROM \"CoexistSyncRun\" WHERE \
             ((\"status\" = 'init' AND \"updatedAt\" < ?2) OR (\"status\" = 'running' AND \
             \"lastHeartbeatAt\" < ?3)) AND \"type\" = ?4 AND \"attempts\" < ?5 \
             ORDER BY \"createdAt\" ASC LIMIT ?6) {PICKED_COLUMNS}"
        ),
        &[
            &now,
            &(now - INIT_DUE_MS),
            &(now - RUNNING_DUE_MS),
            &run_type,
            &max_attempts,
            &batch_size,
        ],
    )
}

/// `EXISTS (pending staging of r's phone number)`.
fn pending_staging_of_run() -> String {
    format!(
        "EXISTS (SELECT 1 FROM \"WhatsappCoexistStaging\" s JOIN \"IntegrationWhatsapp\" i \
         ON i.\"phoneNumberId\" = s.\"phoneNumberId\" WHERE i.\"id\" = r.\"integrationId\" \
         AND {PENDING_STAGING})"
    )
}

/// `reviveWaitingRunsWithPendingStaging`.
pub fn revive_waiting_runs_with_pending_staging(
    conn: &Connection,
    batch_size: i64,
) -> Result<Vec<PickedCoexistRun>> {
    picked(
        conn,
        &format!(
            "UPDATE \"CoexistSyncRun\" SET \"status\" = 'init', \"updatedAt\" = ?1 \
             WHERE \"id\" IN (SELECT r.\"id\" FROM \"CoexistSyncRun\" r WHERE r.\"channel\" = 'whatsapp' \
             AND r.\"status\" = 'waiting' AND r.\"type\" = 'coexist' AND {} \
             ORDER BY r.\"createdAt\" ASC LIMIT ?2) {PICKED_COLUMNS}",
            pending_staging_of_run()
        ),
        &[&now_ms(), &batch_size],
    )
}

/// `finalizeTimedOutWaitingRuns`: the ids of the finalized runs.
pub fn finalize_timed_out_waiting_runs(
    conn: &Connection,
    window_ms: i64,
    current_error: &str,
    batch_size: i64,
) -> Result<Vec<i64>> {
    let now = now_ms();
    let window = window_ms.div_euclid(1000) * 1000;
    let mut stmt = conn.prepare(&format!(
        "UPDATE \"CoexistSyncRun\" SET \"status\" = 'partial', \"currentError\" = ?1, \
         \"finishedAt\" = ?2, \"updatedAt\" = ?2 WHERE \"id\" IN (SELECT r.\"id\" FROM \
         \"CoexistSyncRun\" r WHERE r.\"channel\" = 'whatsapp' AND r.\"status\" = 'waiting' \
         AND r.\"type\" = 'coexist' AND COALESCE(r.\"startedAt\", r.\"createdAt\") < ?3 \
         AND NOT {} ORDER BY r.\"createdAt\" ASC LIMIT ?4) RETURNING \"id\", \"createdAt\"",
        pending_staging_of_run()
    ))?;
    let mut rows = stmt
        .query_map(params![current_error, now, now - window, batch_size], |r| {
            Ok((r.get::<_, i64>(1)?, r.get::<_, i64>(0)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.sort_unstable();
    Ok(rows.into_iter().map(|(_, id)| id).collect())
}

/// `findStrandedCoexistWhatsappIntegrations`.
pub fn find_stranded_coexist_whatsapp_integrations(
    conn: &Connection,
    limit: i64,
) -> Result<Vec<StrandedIntegration>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT i.\"id\", i.\"workspaceId\" FROM \"IntegrationWhatsapp\" i \
         WHERE i.\"coexistEnabled\" = 1 \
         AND EXISTS (SELECT 1 FROM \"WhatsappCoexistStaging\" s \
           WHERE s.\"phoneNumberId\" = i.\"phoneNumberId\" AND {PENDING_STAGING}) \
         AND NOT EXISTS (SELECT 1 FROM \"CoexistSyncRun\" r WHERE r.\"integrationId\" = i.\"id\" \
           AND r.\"channel\" = 'whatsapp' AND r.\"type\" = 'coexist' \
           AND r.\"status\" IN ({LIVE_STATUS_LIST})) \
         ORDER BY i.\"id\" ASC LIMIT ?1"
    ))?;
    let rows = stmt
        .query_map(params![limit], |r| {
            Ok(StrandedIntegration {
                integration_id: r.get(0)?,
                workspace_id: r.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// `UPDATE "CoexistSyncRun" SET <head>, <fields>, "updatedAt" = now WHERE <filter>`.
fn update_run(conn: &Connection, head: Sql, fields: &Patch, filter: Sql) -> Result<usize> {
    let mut q = Sql::raw("UPDATE \"CoexistSyncRun\" SET ");
    let has_head = !head.text.is_empty();
    q.append(head);
    if !fields.is_empty() {
        if has_head {
            q.push(", ");
        }
        q.append(patch::set_clause(PROGRESS_FIELDS, fields)?);
    }
    if has_head || !fields.is_empty() {
        q.push(", ");
    }
    q.push("\"updatedAt\" = ");
    q.push_bind(now_ms());
    q.push(" WHERE ");
    q.append(filter);
    q.execute(conn)
}

/// `updateProgress`: the affected-row count.
pub fn update_progress(
    conn: &Connection,
    run_id: i64,
    fields: &Patch,
    expect: Option<&WriteGuard<'_>>,
) -> Result<usize> {
    update_run(conn, Sql::default(), fields, write_filter(run_id, expect))
}

fn terminal_fields(status: &str, current_error: Option<&str>) -> Patch {
    let mut fields: Patch = vec![("status", Value::Text(status.to_owned()))];
    if let Some(error) = current_error {
        fields.push(("currentError", Value::Text(error.to_owned())));
    }
    fields.push(("finishedAt", Value::Integer(now_ms())));
    fields.push(("resumeCursor", Value::Null));
    fields
}

/// `markFailed`.
pub fn mark_failed(
    conn: &Connection,
    run_id: i64,
    current_error: &str,
    expect: Option<&WriteGuard<'_>>,
) -> Result<usize> {
    update_progress(
        conn,
        run_id,
        &terminal_fields("failed", Some(current_error)),
        expect,
    )
}

/// `markPartial` (`None` leaves `currentError` as it is).
pub fn mark_partial(
    conn: &Connection,
    run_id: i64,
    current_error: Option<&str>,
    expect: Option<&WriteGuard<'_>>,
) -> Result<usize> {
    update_progress(
        conn,
        run_id,
        &terminal_fields("partial", current_error),
        expect,
    )
}

/// `markSucceeded`.
pub fn mark_succeeded(
    conn: &Connection,
    run_id: i64,
    expect: Option<&WriteGuard<'_>>,
) -> Result<usize> {
    update_progress(conn, run_id, &terminal_fields("succeeded", None), expect)
}

/// `findResumeCeiling` (ms). Postgres sorts NULL `startedAt` first under `DESC`; so does this.
pub fn find_resume_ceiling(
    conn: &Connection,
    integration_id: i64,
    channel: &str,
    current_run_id: i64,
) -> Result<Option<i64>> {
    let prior = conn
        .query_row(
            "SELECT \"startedAt\", \"lastSyncedAt\", \"status\" FROM \"CoexistSyncRun\" \
             WHERE \"integrationId\" = ?1 AND \"channel\" = ?2 AND \"status\" IN ('succeeded', 'partial') \
             AND \"id\" <> ?3 AND \"type\" = 'coexist' \
             ORDER BY \"startedAt\" IS NULL DESC, \"startedAt\" DESC LIMIT 1",
            params![integration_id, channel, current_run_id],
            |r| {
                Ok((
                    r.get::<_, Option<i64>>(0)?,
                    r.get::<_, Option<i64>>(1)?,
                    r.get::<_, String>(2)?,
                ))
            },
        )
        .optional()?;
    Ok(prior.and_then(|(started, synced, status)| {
        if status == "succeeded" {
            started
        } else {
            synced.or(started)
        }
    }))
}

/// `tearDownActiveRunsForIntegration`.
pub fn tear_down_active_runs_for_integration(
    conn: &Connection,
    channel: &str,
    integration_id: i64,
    current_error: &str,
) -> Result<()> {
    let now = now_ms();
    conn.execute(
        &format!(
            "UPDATE \"CoexistSyncRun\" SET \"status\" = 'failed', \"currentError\" = ?1, \
             \"finishedAt\" = ?2, \"updatedAt\" = ?2 WHERE \"channel\" = ?3 AND \"integrationId\" = ?4 \
             AND \"type\" = 'coexist' AND \"status\" IN ({LIVE_STATUS_LIST})"
        ),
        params![current_error, now, channel, integration_id],
    )?;
    Ok(())
}

fn unknown_channel(channel: &str) -> Error {
    Error::msg(format!("unknown coexist channel: {channel}"))
}

fn lookup<M: Model>(
    conn: &Connection,
    workspace_id: i64,
    integration_id: i64,
) -> Result<Option<M>> {
    super::find_one::<M>(
        conn,
        "\"id\" = ?1 AND \"workspaceId\" = ?2",
        &[&integration_id, &workspace_id],
    )
}

/// `findIntegrationForCoexist`.
pub fn find_integration_for_coexist(
    conn: &Connection,
    workspace_id: i64,
    integration_id: i64,
    channel: &str,
) -> Result<Option<CoexistIntegrationRow>> {
    Ok(match channel {
        "messenger" => {
            lookup(conn, workspace_id, integration_id)?.map(CoexistIntegrationRow::Messenger)
        }
        "instagram" => {
            lookup(conn, workspace_id, integration_id)?.map(CoexistIntegrationRow::Instagram)
        }
        "whatsapp" => {
            lookup(conn, workspace_id, integration_id)?.map(CoexistIntegrationRow::Whatsapp)
        }
        other => return Err(unknown_channel(other)),
    })
}

fn set_enabled<M: Model>(
    conn: &Connection,
    workspace_id: i64,
    integration_id: i64,
    enabled: bool,
    ai_reads_synced_history: Option<bool>,
) -> Result<Option<M>> {
    let mut q = Sql::bind(
        format!("UPDATE \"{}\" SET \"coexistEnabled\" = ", M::TABLE),
        enabled,
    );
    if let Some(ai) = ai_reads_synced_history {
        q.push(", \"coexistAiReadsSyncedHistory\" = ");
        q.push_bind(ai);
    }
    q.push(", \"updatedAt\" = ");
    q.push_bind(now_ms());
    q.push(" WHERE \"id\" = ");
    q.push_bind(integration_id);
    q.push(" AND \"workspaceId\" = ");
    q.push_bind(workspace_id);
    let params: Vec<&dyn rusqlite::ToSql> =
        q.params.iter().map(|v| v as &dyn rusqlite::ToSql).collect();
    Ok(super::returning::<M>(conn, &q.text, &params)?
        .into_iter()
        .next())
}

/// `setIntegrationCoexistEnabled` (`None` leaves `coexistAiReadsSyncedHistory` as it is).
pub fn set_integration_coexist_enabled(
    conn: &Connection,
    channel: &str,
    workspace_id: i64,
    integration_id: i64,
    enabled: bool,
    ai_reads_synced_history: Option<bool>,
) -> Result<Option<CoexistIntegrationRow>> {
    let (w, i, e, a) = (
        workspace_id,
        integration_id,
        enabled,
        ai_reads_synced_history,
    );
    Ok(match channel {
        "messenger" => set_enabled(conn, w, i, e, a)?.map(CoexistIntegrationRow::Messenger),
        "instagram" => set_enabled(conn, w, i, e, a)?.map(CoexistIntegrationRow::Instagram),
        "whatsapp" => set_enabled(conn, w, i, e, a)?.map(CoexistIntegrationRow::Whatsapp),
        other => return Err(unknown_channel(other)),
    })
}

/// `findLastSyncedAt`: `None` for no run, `Some(lastSyncedAt)` otherwise.
pub fn find_last_synced_at(conn: &Connection, run_id: i64) -> Result<Option<Option<i64>>> {
    Ok(conn
        .query_row(
            "SELECT \"lastSyncedAt\" FROM \"CoexistSyncRun\" WHERE \"id\" = ?1",
            params![run_id],
            |r| r.get::<_, Option<i64>>(0),
        )
        .optional()?)
}

/// `incrementProgress`: atomic `col + n` counters plus `fields`; the affected-row count.
/// A `fields` key overrides the counter of the same column (the TS object spread).
pub fn increment_progress(
    conn: &Connection,
    run_id: i64,
    increments: &Increments,
    fields: &Patch,
    expect: Option<&WriteGuard<'_>>,
) -> Result<usize> {
    let counters = [
        ("currentScan", increments.current_scan),
        ("importedContactCount", increments.imported_contact_count),
        ("importedMessageCount", increments.imported_message_count),
        ("skippedCount", increments.skipped_count),
        ("failedCount", increments.failed_count),
    ];
    let mut head = Sql::default();
    for (column, amount) in counters {
        let Some(amount) = amount else { continue };
        if fields.iter().any(|(c, _)| *c == column) {
            continue;
        }
        if !head.text.is_empty() {
            head.push(", ");
        }
        head.push(&format!("\"{column}\" = \"{column}\" + "));
        head.push_bind(amount);
    }
    update_run(conn, head, fields, write_filter(run_id, expect))
}

/// `findInitState`.
pub fn find_init_state(conn: &Connection, run_id: i64) -> Result<Option<InitState>> {
    Ok(conn
        .query_row(
            "SELECT \"attempts\", \"currentError\", \"messengerSyncPhase\" FROM \"CoexistSyncRun\" \
             WHERE \"id\" = ?1",
            params![run_id],
            |r| {
                Ok(InitState {
                    attempts: r.get(0)?,
                    current_error: r.get(1)?,
                    messenger_sync_phase: r.get(2)?,
                })
            },
        )
        .optional()?)
}

/// `reclaimRunForRetry`.
pub fn reclaim_run_for_retry(
    conn: &Connection,
    run_id: i64,
    touch_updated_at: bool,
) -> Result<Option<CoexistSyncRun>> {
    let now = now_ms();
    let touch = if touch_updated_at {
        ", \"updatedAt\" = ?2"
    } else {
        ""
    };
    let rows = super::returning::<CoexistSyncRun>(
        conn,
        &format!(
            "UPDATE \"CoexistSyncRun\" SET \"status\" = 'running', \
             \"startedAt\" = COALESCE(\"startedAt\", ?2), \"lastHeartbeatAt\" = ?2{touch} \
             WHERE \"id\" = ?1 AND (\"status\" <> 'running' OR \"lastHeartbeatAt\" < ?3)"
        ),
        &[&run_id, &now, &(now - STALE_CLAIM_MS)],
    )?;
    Ok(rows.into_iter().next())
}

/// `findTerminalCounters`.
pub fn find_terminal_counters(conn: &Connection, run_id: i64) -> Result<Option<TerminalCounters>> {
    Ok(conn
        .query_row(
            "SELECT \"importedMessageCount\", \"skippedCount\", \"failedCount\" FROM \"CoexistSyncRun\" \
             WHERE \"id\" = ?1",
            params![run_id],
            |r| {
                Ok(TerminalCounters {
                    imported_message_count: r.get(0)?,
                    skipped_count: r.get(1)?,
                    failed_count: r.get(2)?,
                })
            },
        )
        .optional()?)
}

/// `claimContactScanRun`.
pub fn claim_contact_scan_run(conn: &Connection, run_id: i64) -> Result<Option<CoexistSyncRun>> {
    let now = now_ms();
    let rows = super::returning::<CoexistSyncRun>(
        conn,
        "UPDATE \"CoexistSyncRun\" SET \"status\" = 'running', \"claimToken\" = ?2, \
         \"startedAt\" = COALESCE(\"startedAt\", ?3), \"lastHeartbeatAt\" = ?3, \"updatedAt\" = ?3 \
         WHERE \"id\" = ?1 AND \"type\" = 'contact_scan' AND \"status\" IN ('init', 'running') \
         AND (\"status\" <> 'running' OR \"claimToken\" IS NULL OR \"lastHeartbeatAt\" < ?4)",
        &[&run_id, &random_uuid(), &now, &(now - STALE_CLAIM_MS)],
    )?;
    Ok(rows.into_iter().next())
}

/// `yieldForContinuation`.
pub fn yield_for_continuation(
    conn: &Connection,
    run_id: i64,
    expect: &WriteGuard<'_>,
) -> Result<usize> {
    let mut head = Sql::raw("\"claimToken\" = NULL, \"lastHeartbeatAt\" = ");
    head.push_bind(now_ms());
    let mut filter = Sql::raw("\"type\" = 'contact_scan' AND ");
    filter.append(write_filter(run_id, Some(expect)));
    update_run(conn, head, &Vec::new(), filter)
}

/// `reopenReleased`.
pub fn reopen_released(conn: &Connection, run_id: i64) -> Result<usize> {
    Ok(conn.execute(
        "UPDATE \"CoexistSyncRun\" SET \"status\" = 'init', \"updatedAt\" = ?2 \
         WHERE \"id\" = ?1 AND \"type\" = 'contact_scan' AND \"status\" = 'running' \
         AND \"claimToken\" IS NULL",
        params![run_id, now_ms()],
    )?)
}

/// `createContactScanRun`: `None` when the active-scan unique index refused the row.
pub fn create_contact_scan_run(
    conn: &Connection,
    input: &CreateContactScanRunInput<'_>,
) -> Result<Option<CoexistSyncRun>> {
    let rows = super::returning::<CoexistSyncRun>(
        conn,
        "INSERT INTO \"CoexistSyncRun\" (\"id\", \"createdAt\", \"updatedAt\", \"workspaceId\", \
         \"integrationId\", \"channel\", \"type\", \"status\", \"triggerSource\", \"scanFromAt\", \
         \"requestedByUserId\", \"pendingPatches\", \"claimToken\", \"resumeCursor\") \
         VALUES (?1, ?2, ?2, ?3, ?4, ?5, 'contact_scan', 'init', ?6, ?7, ?8, NULL, NULL, NULL) \
         ON CONFLICT DO NOTHING",
        &[
            &new_id(),
            &now_ms(),
            &input.workspace_id,
            &input.integration_id,
            &input.channel,
            &input.trigger_source,
            &input.scan_from_at,
            &input.requested_by_user_id,
        ],
    )?;
    Ok(rows.into_iter().next())
}

/// `findLatestContactScanRun`.
pub fn find_latest_contact_scan_run(
    conn: &Connection,
    workspace_id: i64,
    integration_id: i64,
) -> Result<Option<CoexistSyncRun>> {
    first(
        conn,
        "\"workspaceId\" = ?1 AND \"integrationId\" = ?2 AND \"type\" = 'contact_scan' \
         ORDER BY \"createdAt\" DESC, \"id\" DESC",
        &[&workspace_id, &integration_id],
    )
}

/// `listContactScanRuns`.
pub fn list_contact_scan_runs(
    conn: &Connection,
    input: &ListContactScanRunsInput,
) -> Result<ContactScanRunPage> {
    let pagination = get_pagination_with_defaults(PaginationInput {
        page: input.page,
        per_page: input.per_page,
    });
    let mut order = parse_order_by(CoexistSyncRun::COLUMNS, input.sort.as_deref(), None);
    if order.is_empty() {
        order = vec![("createdAt".to_owned(), SortDir::Desc)];
    }
    let mut filter = Sql::bind("\"workspaceId\" = ", input.workspace_id);
    filter.push(" AND \"type\" = 'contact_scan'");
    if let Some(integration_id) = input.integration_id {
        filter.push(" AND \"integrationId\" = ");
        filter.push_bind(integration_id);
    }
    let mut q = Sql::raw(format!("{} WHERE ", CoexistSyncRun::select_sql()));
    q.append(filter.clone());
    q.push(" ");
    q.push(&order_by_sql(&order, None));
    q.push(" LIMIT ");
    q.push_bind(pagination.limit);
    q.push(" OFFSET ");
    q.push_bind(pagination.offset);
    let data = q.models(conn)?;
    let mut count = Sql::raw("SELECT count(*) FROM \"CoexistSyncRun\" WHERE ");
    count.append(filter);
    let total = count
        .query_map(conn, |r| r.get::<_, i64>(0))?
        .into_iter()
        .next()
        .unwrap_or(0);
    let page_count = if pagination.limit > 0 {
        (total + pagination.limit - 1) / pagination.limit
    } else {
        0
    };
    Ok(ContactScanRunPage { data, page_count })
}
