//! Port of `repositories/facebook-marketing-messages/recipient-repository.ts` (P3.2b).

use rusqlite::Connection;

use super::Result;
use super::models::FacebookMarketingMessageRecipient;
use super::patch::Patch;

/// `failRemaining` input (`statuses` defaults to `pending` and `unconfirmed`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FailRemainingInput {
    pub campaign_row_id: i64,
    pub statuses: Option<Vec<String>>,
    pub error_code: Option<i64>,
    pub error_message: String,
    pub error_source: Option<String>,
    pub now: i64,
}

/// `countOutcomes`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Outcomes {
    pub pending: i64,
    pub unconfirmed: i64,
    pub sent: i64,
    pub delivered: i64,
    pub failed: i64,
}

/// One `errorBreakdown` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorBreakdownRow {
    pub error_code: Option<i64>,
    pub error_message: String,
    pub count: i64,
}

/// `insertMany`: the number of rows inserted (duplicates are skipped).
pub fn insert_many(_conn: &Connection, _rows: &[Patch]) -> Result<usize> {
    todo!("P3.2b")
}

/// `listDue`.
pub fn list_due(
    _conn: &Connection,
    _campaign_row_id: i64,
    _now: i64,
    _limit: i64,
) -> Result<Vec<FacebookMarketingMessageRecipient>> {
    todo!("P3.2b")
}

/// `update`.
pub fn update(_conn: &Connection, _id: i64, _patch: &Patch) -> Result<()> {
    todo!("P3.2b")
}

/// `settleSend`: only while the row is still `unconfirmed`.
pub fn settle_send(_conn: &Connection, _id: i64, _patch: &Patch) -> Result<()> {
    todo!("P3.2b")
}

/// `claimForSend`: pending -> unconfirmed for a due row of a `sending` campaign.
pub fn claim_for_send(
    _conn: &Connection,
    _id: i64,
    _now: i64,
) -> Result<Option<FacebookMarketingMessageRecipient>> {
    todo!("P3.2b")
}

/// `failStaleUnconfirmed`: the count.
pub fn fail_stale_unconfirmed(
    _conn: &Connection,
    _campaign_row_id: i64,
    _older_than: i64,
    _now: i64,
) -> Result<usize> {
    todo!("P3.2b")
}

/// `failRemaining`: the count.
pub fn fail_remaining(_conn: &Connection, _input: &FailRemainingInput) -> Result<usize> {
    todo!("P3.2b")
}

/// `findByTrackingId`.
pub fn find_by_tracking_id(
    _conn: &Connection,
    _tracking_id: &str,
) -> Result<Option<FacebookMarketingMessageRecipient>> {
    todo!("P3.2b")
}

/// `findByCampaignToken`.
pub fn find_by_campaign_token(
    _conn: &Connection,
    _campaign_row_id: i64,
    _subscription_token: &str,
) -> Result<Option<FacebookMarketingMessageRecipient>> {
    todo!("P3.2b")
}

/// `countOutcomes`.
pub fn count_outcomes(_conn: &Connection, _campaign_row_id: i64) -> Result<Outcomes> {
    todo!("P3.2b")
}

/// `errorBreakdown`: failed rows by code (else by message), top 50.
pub fn error_breakdown(
    _conn: &Connection,
    _campaign_row_id: i64,
) -> Result<Vec<ErrorBreakdownRow>> {
    todo!("P3.2b")
}
