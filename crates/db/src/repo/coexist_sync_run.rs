//! Port of `repositories/coexist-sync-run/repository.ts` and `integration-access.ts` (P3.2b).
//!
//! `NOW()` is the wall clock in ms ([`now_ms`](super::now_ms)), bound once per statement.
//! `FOR UPDATE SKIP LOCKED` has no SQLite form: one writer runs at a time.

#![allow(unused_variables)]

use rusqlite::Connection;

use super::Result;
use super::models::{
    CoexistSyncRun, IntegrationInstagram, IntegrationMessenger, IntegrationWhatsapp,
};
use super::patch::Patch;
use super::utils::SortItem;

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
        todo!("P3.2b")
    }

    /// `id` of the row.
    pub fn id(&self) -> i64 {
        todo!("P3.2b")
    }

    /// `coexistEnabled` of the row.
    pub fn coexist_enabled(&self) -> bool {
        todo!("P3.2b")
    }

    /// `coexistAiReadsSyncedHistory` of the row.
    pub fn coexist_ai_reads_synced_history(&self) -> bool {
        todo!("P3.2b")
    }
}

/// `createRun`: inserts an `init` coexist run; on a conflict, the live `init` run.
pub fn create_run(conn: &Connection, input: &CreateRunInput<'_>) -> Result<CoexistSyncRun> {
    todo!("P3.2b")
}

/// `findActiveInitRun`.
pub fn find_active_init_run(
    conn: &Connection,
    integration_id: i64,
    channel: &str,
) -> Result<Option<CoexistSyncRun>> {
    todo!("P3.2b")
}

/// `findLatestRun`.
pub fn find_latest_run(
    conn: &Connection,
    workspace_id: i64,
    integration_id: i64,
    channel: &str,
) -> Result<Option<CoexistSyncRun>> {
    todo!("P3.2b")
}

/// `findLiveRun`.
pub fn find_live_run(
    conn: &Connection,
    integration_id: i64,
    channel: &str,
) -> Result<Option<CoexistSyncRun>> {
    todo!("P3.2b")
}

/// `findRunById`.
pub fn find_run_by_id(conn: &Connection, run_id: i64) -> Result<Option<CoexistSyncRun>> {
    todo!("P3.2b")
}

/// `findWorkspaceOwnerId`.
pub fn find_workspace_owner_id(conn: &Connection, workspace_id: i64) -> Result<Option<i64>> {
    todo!("P3.2b")
}

/// `claimRunWithNewToken` (`from_statuses` defaults to [`PULL_CLAIMABLE_STATUSES`]).
pub fn claim_run_with_new_token(
    conn: &Connection,
    run_id: i64,
    from_statuses: Option<&[&str]>,
) -> Result<Option<CoexistSyncRun>> {
    todo!("P3.2b")
}

/// `markMaxAttemptsFailed`.
pub fn mark_max_attempts_failed(
    conn: &Connection,
    run_type: &str,
    max_attempts: i64,
) -> Result<()> {
    todo!("P3.2b")
}

/// `pickDueRuns`.
pub fn pick_due_runs(
    conn: &Connection,
    run_type: &str,
    batch_size: i64,
    max_attempts: i64,
) -> Result<Vec<PickedCoexistRun>> {
    todo!("P3.2b")
}

/// `reviveWaitingRunsWithPendingStaging`.
pub fn revive_waiting_runs_with_pending_staging(
    conn: &Connection,
    batch_size: i64,
) -> Result<Vec<PickedCoexistRun>> {
    todo!("P3.2b")
}

/// `finalizeTimedOutWaitingRuns`: the ids of the finalized runs.
pub fn finalize_timed_out_waiting_runs(
    conn: &Connection,
    window_ms: i64,
    current_error: &str,
    batch_size: i64,
) -> Result<Vec<i64>> {
    todo!("P3.2b")
}

/// `findStrandedCoexistWhatsappIntegrations`.
pub fn find_stranded_coexist_whatsapp_integrations(
    conn: &Connection,
    limit: i64,
) -> Result<Vec<StrandedIntegration>> {
    todo!("P3.2b")
}

/// `updateProgress`: the affected-row count.
pub fn update_progress(
    conn: &Connection,
    run_id: i64,
    fields: &Patch,
    expect: Option<&WriteGuard<'_>>,
) -> Result<usize> {
    todo!("P3.2b")
}

/// `markFailed`.
pub fn mark_failed(
    conn: &Connection,
    run_id: i64,
    current_error: &str,
    expect: Option<&WriteGuard<'_>>,
) -> Result<usize> {
    todo!("P3.2b")
}

/// `markPartial` (`None` leaves `currentError` as it is).
pub fn mark_partial(
    conn: &Connection,
    run_id: i64,
    current_error: Option<&str>,
    expect: Option<&WriteGuard<'_>>,
) -> Result<usize> {
    todo!("P3.2b")
}

/// `markSucceeded`.
pub fn mark_succeeded(
    conn: &Connection,
    run_id: i64,
    expect: Option<&WriteGuard<'_>>,
) -> Result<usize> {
    todo!("P3.2b")
}

/// `findResumeCeiling` (ms).
pub fn find_resume_ceiling(
    conn: &Connection,
    integration_id: i64,
    channel: &str,
    current_run_id: i64,
) -> Result<Option<i64>> {
    todo!("P3.2b")
}

/// `tearDownActiveRunsForIntegration`.
pub fn tear_down_active_runs_for_integration(
    conn: &Connection,
    channel: &str,
    integration_id: i64,
    current_error: &str,
) -> Result<()> {
    todo!("P3.2b")
}

/// `findIntegrationForCoexist`.
pub fn find_integration_for_coexist(
    conn: &Connection,
    workspace_id: i64,
    integration_id: i64,
    channel: &str,
) -> Result<Option<CoexistIntegrationRow>> {
    todo!("P3.2b")
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
    todo!("P3.2b")
}

/// `findLastSyncedAt`: `None` for no run, `Some(lastSyncedAt)` otherwise.
pub fn find_last_synced_at(conn: &Connection, run_id: i64) -> Result<Option<Option<i64>>> {
    todo!("P3.2b")
}

/// `incrementProgress`: atomic `col + n` counters plus `fields`; the affected-row count.
pub fn increment_progress(
    conn: &Connection,
    run_id: i64,
    increments: &Increments,
    fields: &Patch,
    expect: Option<&WriteGuard<'_>>,
) -> Result<usize> {
    todo!("P3.2b")
}

/// `findInitState`.
pub fn find_init_state(conn: &Connection, run_id: i64) -> Result<Option<InitState>> {
    todo!("P3.2b")
}

/// `reclaimRunForRetry`.
pub fn reclaim_run_for_retry(
    conn: &Connection,
    run_id: i64,
    touch_updated_at: bool,
) -> Result<Option<CoexistSyncRun>> {
    todo!("P3.2b")
}

/// `findTerminalCounters`.
pub fn find_terminal_counters(conn: &Connection, run_id: i64) -> Result<Option<TerminalCounters>> {
    todo!("P3.2b")
}

/// `claimContactScanRun`.
pub fn claim_contact_scan_run(conn: &Connection, run_id: i64) -> Result<Option<CoexistSyncRun>> {
    todo!("P3.2b")
}

/// `yieldForContinuation`.
pub fn yield_for_continuation(
    conn: &Connection,
    run_id: i64,
    expect: &WriteGuard<'_>,
) -> Result<usize> {
    todo!("P3.2b")
}

/// `reopenReleased`.
pub fn reopen_released(conn: &Connection, run_id: i64) -> Result<usize> {
    todo!("P3.2b")
}

/// `createContactScanRun`: `None` when the active-scan unique index refused the row.
pub fn create_contact_scan_run(
    conn: &Connection,
    input: &CreateContactScanRunInput<'_>,
) -> Result<Option<CoexistSyncRun>> {
    todo!("P3.2b")
}

/// `findLatestContactScanRun`.
pub fn find_latest_contact_scan_run(
    conn: &Connection,
    workspace_id: i64,
    integration_id: i64,
) -> Result<Option<CoexistSyncRun>> {
    todo!("P3.2b")
}

/// `listContactScanRuns`.
pub fn list_contact_scan_runs(
    conn: &Connection,
    input: &ListContactScanRunsInput,
) -> Result<ContactScanRunPage> {
    todo!("P3.2b")
}
