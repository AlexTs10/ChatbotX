//! Port of `repositories/google-ads-conversion-event/repository.ts` (P3.2b).
//!
//! Instants are unix ms. `value` is the exact decimal text of the `numeric` column.

use rusqlite::Connection;

use super::Result;
use super::models::GoogleAdsConversionEvent;
use super::patch::Patch;

/// `GOOGLE_ADS_EVENTS_MAX_PER_PAGE` (`partials/google-ads.ts`).
pub const GOOGLE_ADS_EVENTS_MAX_PER_PAGE: i64 = 100;

/// `EventRef`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventRef {
    pub id: i64,
    pub workspace_id: i64,
}

/// `ClaimedEventRef & { attempt }` (`claimForSending`, `markSendAttempted`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimInput {
    pub id: i64,
    pub workspace_id: i64,
    pub claim_token: String,
    pub attempt: i64,
}

/// `FinishSendingInput` (`to` = `sent` / `failed` / `skipped_no_account` / `skipped_expired`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FinishSendingInput {
    pub id: i64,
    pub workspace_id: i64,
    pub claim_token: String,
    pub to: String,
    pub request_id: Option<String>,
    pub sent_at: Option<i64>,
    pub error: Option<String>,
    pub failure_stage: Option<String>,
    pub processing_status: Option<String>,
    pub next_processing_check_at: Option<i64>,
}

/// `FinishSendingProcessedInput`.
#[derive(Debug, Clone, PartialEq)]
pub struct FinishSendingProcessedInput {
    pub id: i64,
    pub workspace_id: i64,
    pub claim_token: String,
    pub attempt: i64,
    pub request_id: String,
    pub sent_at: i64,
    pub processing_detail: serde_json::Value,
}

/// `redrive` input. `expected_claim_token`: `None` = no fence, `Some(None)` = unleased.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RedriveInput {
    pub id: i64,
    pub workspace_id: i64,
    pub from_statuses: Vec<String>,
    pub expected_attempt: i64,
    pub expected_request_id: Option<String>,
    pub expected_claim_token: Option<Option<String>>,
}

/// `ApplyProcessingResultInput` (`to` = `sent` / `processed` / `failed`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ApplyProcessingResultInput {
    pub id: i64,
    pub workspace_id: i64,
    pub to: String,
    pub processing_status: String,
    pub processing_detail: Option<serde_json::Value>,
    pub failure_stage: Option<String>,
    pub error: Option<String>,
    pub next_processing_check_at: Option<i64>,
    pub processing_attempts: i64,
    pub expected_request_id: Option<String>,
    pub expected_attempt: i64,
}

/// `ListGoogleAdsEventsInput` (`per_page` is clamped to [`GOOGLE_ADS_EVENTS_MAX_PER_PAGE`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListInput {
    pub workspace_id: i64,
    pub status: Option<String>,
    pub channel: Option<String>,
    pub conversion_action_id: Option<String>,
    pub since: Option<i64>,
    pub until: Option<i64>,
    pub page: i64,
    pub per_page: i64,
}

/// `listByWorkspace` result.
#[derive(Debug, Clone, PartialEq)]
pub struct ListResult {
    pub rows: Vec<GoogleAdsConversionEvent>,
    pub total: i64,
}

/// `GoogleAdsStatsFilters` (bounds inclusive).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StatsFilters {
    pub workspace_id: i64,
    pub channel: Option<String>,
    pub conversion_action_id: Option<String>,
    pub since: i64,
    pub until: i64,
}

/// One count per event status.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StatusCounts {
    pub pending: i64,
    pub sending: i64,
    pub sent: i64,
    pub processed: i64,
    pub failed: i64,
    pub skipped_no_account: i64,
    pub skipped_expired: i64,
}

/// `GoogleAdsDayChannelStatsRow` (`date` first, as the TS select).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DayChannelStatsRow {
    pub date: String,
    pub channel: String,
    pub counts: StatusCounts,
    pub failed_delivery: i64,
    pub failed_processing: i64,
    pub failed_timeout: i64,
    pub failed_unknown: i64,
}

/// `GoogleAdsActionStatsRow`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ActionStatsRow {
    pub conversion_action_id: String,
    pub name: Option<String>,
    pub category: Option<String>,
    pub counts: StatusCounts,
}

/// `GoogleAdsConfirmedValueRow` (`value` is exact decimal text).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmedValueRow {
    pub currency: Option<String>,
    pub value: String,
    pub count: i64,
}

/// `listStranded` input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StrandedInput {
    pub pending_older_than: i64,
    pub six_hour_gate_before: i64,
    pub sending_older_than: i64,
    pub limit: i64,
}

/// `insertIgnoreDuplicate`: `values` must carry `options` (the TS type requires it).
pub fn insert_ignore_duplicate(
    _conn: &Connection,
    _values: &Patch,
) -> Result<Option<GoogleAdsConversionEvent>> {
    todo!("P3.2b")
}

/// `findByTransactionId`.
pub fn find_by_transaction_id(
    _conn: &Connection,
    _workspace_id: i64,
    _transaction_id: &str,
) -> Result<Option<GoogleAdsConversionEvent>> {
    todo!("P3.2b")
}

/// `findWorkspaceEvent`.
pub fn find_workspace_event(
    _conn: &Connection,
    _event: EventRef,
) -> Result<Option<GoogleAdsConversionEvent>> {
    todo!("P3.2b")
}

/// `claimForSending`.
pub fn claim_for_sending(
    _conn: &Connection,
    _input: &ClaimInput,
) -> Result<Option<GoogleAdsConversionEvent>> {
    todo!("P3.2b")
}

/// `finishSending`.
pub fn finish_sending(
    _conn: &Connection,
    _input: &FinishSendingInput,
) -> Result<Option<GoogleAdsConversionEvent>> {
    todo!("P3.2b")
}

/// `markSendAttempted` (`at` = unix ms).
pub fn mark_send_attempted(
    _conn: &Connection,
    _input: &ClaimInput,
    _at: i64,
) -> Result<Option<GoogleAdsConversionEvent>> {
    todo!("P3.2b")
}

/// `finishSendingProcessed`.
pub fn finish_sending_processed(
    _conn: &Connection,
    _input: &FinishSendingProcessedInput,
) -> Result<Option<GoogleAdsConversionEvent>> {
    todo!("P3.2b")
}

/// `releaseClaim`.
pub fn release_claim(
    _conn: &Connection,
    _event: EventRef,
    _claim_token: &str,
    _next_attempt: i64,
) -> Result<Option<GoogleAdsConversionEvent>> {
    todo!("P3.2b")
}

/// `redrive`.
pub fn redrive(
    _conn: &Connection,
    _input: &RedriveInput,
) -> Result<Option<GoogleAdsConversionEvent>> {
    todo!("P3.2b")
}

/// `applyProcessingResult`.
pub fn apply_processing_result(
    _conn: &Connection,
    _input: &ApplyProcessingResultInput,
) -> Result<Option<GoogleAdsConversionEvent>> {
    todo!("P3.2b")
}

/// `listByWorkspace`.
pub fn list_by_workspace(_conn: &Connection, _input: &ListInput) -> Result<ListResult> {
    todo!("P3.2b")
}

/// `statsByDayAndChannel`.
pub fn stats_by_day_and_channel(
    _conn: &Connection,
    _filters: &StatsFilters,
    _timezone: &str,
) -> Result<Vec<DayChannelStatsRow>> {
    todo!("P3.2b")
}

/// `statsByAction`: busiest first, `limit + 1` rows.
pub fn stats_by_action(
    _conn: &Connection,
    _filters: &StatsFilters,
    _limit: i64,
) -> Result<Vec<ActionStatsRow>> {
    todo!("P3.2b")
}

/// `confirmedValueByCurrency`.
pub fn confirmed_value_by_currency(
    _conn: &Connection,
    _filters: &StatsFilters,
) -> Result<Vec<ConfirmedValueRow>> {
    todo!("P3.2b")
}

/// `existsForWorkspace`.
pub fn exists_for_workspace(_conn: &Connection, _workspace_id: i64) -> Result<bool> {
    todo!("P3.2b")
}

/// `listDueForProcessingCheck`.
pub fn list_due_for_processing_check(
    _conn: &Connection,
    _now: i64,
    _limit: i64,
) -> Result<Vec<GoogleAdsConversionEvent>> {
    todo!("P3.2b")
}

/// `touchStranded`.
pub fn touch_stranded(
    _conn: &Connection,
    _event: EventRef,
    _status: &str,
    _attempt: i64,
) -> Result<()> {
    todo!("P3.2b")
}

/// `listStranded`: stale `sending` rows, then stale `pending` rows (each window limited).
pub fn list_stranded(
    _conn: &Connection,
    _input: &StrandedInput,
) -> Result<Vec<GoogleAdsConversionEvent>> {
    todo!("P3.2b")
}
