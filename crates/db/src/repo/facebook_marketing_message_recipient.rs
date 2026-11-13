//! Port of `repositories/facebook-marketing-messages/recipient-repository.ts` (P3.2b).

use rusqlite::Connection;
use rusqlite::types::Value;

use super::models::FacebookMarketingMessageRecipient;
use super::patch::{self, Patch};
use super::sql::Sql;
use super::{Error, Model, Result, new_id, now_ms};

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

const TABLE: &str = "\"FacebookMarketingMessageRecipient\"";

/// A `RecipientPatch` may not set these columns.
const FROZEN: &[&str] = &["id", "workspaceId", "campaignRowId", "subscriptionToken"];

/// `UPDATE .. SET <patch>, updatedAt WHERE "id" = ? <extra>` for a `RecipientPatch`.
fn update_where(conn: &Connection, id: i64, patch: &Patch, extra: &str) -> Result<()> {
    if let Some((col, _)) = patch.iter().find(|(c, _)| FROZEN.contains(c)) {
        return Err(Error::msg(format!("recipient patch may not set {col}")));
    }
    let mut set = patch.clone();
    if !set.iter().any(|(c, _)| *c == "updatedAt") {
        set.push(("updatedAt", Value::Integer(now_ms())));
    }
    let mut q = Sql::raw(format!("UPDATE {TABLE} SET "));
    q.append(patch::set_clause(
        FacebookMarketingMessageRecipient::COLUMNS,
        &set,
    )?);
    q.push(" WHERE \"id\" = ");
    q.push_bind(id);
    q.push(extra);
    q.execute(conn)?;
    Ok(())
}

/// `insertMany`: the number of rows inserted (duplicates are skipped).
pub fn insert_many(conn: &Connection, rows: &[Patch]) -> Result<usize> {
    let mut inserted = 0;
    for row in rows {
        let mut values: Patch = Vec::with_capacity(row.len() + 1);
        if !row.iter().any(|(c, _)| *c == "id") {
            values.push(("id", Value::Integer(new_id())));
        }
        values.extend(row.iter().cloned());
        let mut q = Sql::raw(format!("INSERT INTO {TABLE} "));
        q.append(patch::insert_clause(
            FacebookMarketingMessageRecipient::COLUMNS,
            &values,
        )?);
        q.push(" ON CONFLICT DO NOTHING");
        inserted += q.execute(conn)?;
    }
    Ok(inserted)
}

/// `listDue`.
pub fn list_due(
    conn: &Connection,
    campaign_row_id: i64,
    now: i64,
    limit: i64,
) -> Result<Vec<FacebookMarketingMessageRecipient>> {
    super::find_many(
        conn,
        "WHERE \"campaignRowId\" = ?1 AND \"status\" = 'pending' AND (\"nextAttemptAt\" IS NULL \
         OR \"nextAttemptAt\" <= ?2) ORDER BY \"id\" ASC LIMIT ?3",
        &[&campaign_row_id, &now, &limit],
    )
}

/// `update`.
pub fn update(conn: &Connection, id: i64, patch: &Patch) -> Result<()> {
    update_where(conn, id, patch, "")
}

/// `settleSend`: only while the row is still `unconfirmed`.
pub fn settle_send(conn: &Connection, id: i64, patch: &Patch) -> Result<()> {
    update_where(conn, id, patch, " AND \"status\" = 'unconfirmed'")
}

/// `claimForSend`: pending -> unconfirmed for a due row of a `sending` campaign.
pub fn claim_for_send(
    conn: &Connection,
    id: i64,
    now: i64,
) -> Result<Option<FacebookMarketingMessageRecipient>> {
    Ok(super::returning::<FacebookMarketingMessageRecipient>(
        conn,
        &format!(
            "UPDATE {TABLE} SET \"status\" = 'unconfirmed', \"sentAt\" = ?1, \
             \"attempts\" = \"attempts\" + 1, \"updatedAt\" = ?2 WHERE \"id\" = ?3 \
             AND \"status\" = 'pending' AND (\"nextAttemptAt\" IS NULL OR \"nextAttemptAt\" <= ?1) \
             AND EXISTS (SELECT 1 FROM \"FacebookMarketingMessage\" AS c WHERE c.\"id\" = {TABLE}.\"campaignRowId\" \
             AND c.\"status\" = 'sending')"
        ),
        &[&now, &now_ms(), &id],
    )?
    .into_iter()
    .next())
}

/// `failStaleUnconfirmed`: the count.
pub fn fail_stale_unconfirmed(
    conn: &Connection,
    campaign_row_id: i64,
    older_than: i64,
    now: i64,
) -> Result<usize> {
    Ok(conn.execute(
        &format!(
            "UPDATE {TABLE} SET \"status\" = 'failed', \"errorCode\" = NULL, \
             \"errorMessage\" = 'outcomeUnknown', \"errorSource\" = 'send', \"failedAt\" = ?1, \
             \"updatedAt\" = ?2 WHERE \"campaignRowId\" = ?3 AND \"status\" = 'unconfirmed' \
             AND coalesce(\"sentAt\", \"updatedAt\") <= ?4"
        ),
        [now, now_ms(), campaign_row_id, older_than],
    )?)
}

/// `failRemaining`: the count.
pub fn fail_remaining(conn: &Connection, input: &FailRemainingInput) -> Result<usize> {
    let statuses: Vec<String> = input
        .statuses
        .clone()
        .unwrap_or_else(|| vec!["pending".to_owned(), "unconfirmed".to_owned()]);
    if statuses.is_empty() {
        return Ok(0);
    }
    let mut q = Sql::raw(format!(
        "UPDATE {TABLE} SET \"status\" = 'failed', \"errorCode\" = "
    ));
    q.push_bind(super::sql::opt_int(input.error_code));
    q.push(", \"errorMessage\" = ");
    q.push_bind(input.error_message.clone());
    q.push(", \"errorSource\" = ");
    q.push_bind(super::sql::opt_text(input.error_source.as_deref()));
    q.push(", \"failedAt\" = ");
    q.push_bind(input.now);
    q.push(", \"updatedAt\" = ");
    q.push_bind(now_ms());
    q.push(" WHERE \"campaignRowId\" = ");
    q.push_bind(input.campaign_row_id);
    q.append(Sql::new(
        format!(
            " AND \"status\" IN ({})",
            vec!["?"; statuses.len()].join(", ")
        ),
        statuses.into_iter().map(Value::Text).collect(),
    ));
    q.execute(conn)
}

/// `findByTrackingId`.
pub fn find_by_tracking_id(
    conn: &Connection,
    tracking_id: &str,
) -> Result<Option<FacebookMarketingMessageRecipient>> {
    super::find_one(conn, "\"trackingId\" = ?1", &[&tracking_id])
}

/// `findByCampaignToken`.
pub fn find_by_campaign_token(
    conn: &Connection,
    campaign_row_id: i64,
    subscription_token: &str,
) -> Result<Option<FacebookMarketingMessageRecipient>> {
    super::find_one(
        conn,
        "\"campaignRowId\" = ?1 AND \"subscriptionToken\" = ?2",
        &[&campaign_row_id, &subscription_token],
    )
}

/// `countOutcomes`.
pub fn count_outcomes(conn: &Connection, campaign_row_id: i64) -> Result<Outcomes> {
    Ok(conn.query_row(
        &format!(
            "SELECT COALESCE(SUM(\"status\" = 'pending'), 0), COALESCE(SUM(\"status\" = 'unconfirmed'), 0), \
             COALESCE(SUM(\"status\" = 'sent'), 0), COALESCE(SUM(\"status\" = 'delivered'), 0), \
             COALESCE(SUM(\"status\" = 'failed'), 0) FROM {TABLE} WHERE \"campaignRowId\" = ?1"
        ),
        [campaign_row_id],
        |r| {
            Ok(Outcomes {
                pending: r.get(0)?,
                unconfirmed: r.get(1)?,
                sent: r.get(2)?,
                delivered: r.get(3)?,
                failed: r.get(4)?,
            })
        },
    )?)
}

/// `errorBreakdown`: failed rows by code (else by message), top 50.
pub fn error_breakdown(conn: &Connection, campaign_row_id: i64) -> Result<Vec<ErrorBreakdownRow>> {
    Sql::bind(
        format!(
            "SELECT \"errorCode\", coalesce(max(\"errorMessage\"), ''), count(*) FROM {TABLE} \
             WHERE \"status\" = 'failed' AND \"campaignRowId\" = "
        ),
        campaign_row_id,
    )
    .closed(
        " GROUP BY \"errorCode\", CASE WHEN \"errorCode\" IS NULL THEN \"errorMessage\" END \
         ORDER BY count(*) DESC LIMIT 50",
    )
    .query_map(conn, |r| {
        Ok(ErrorBreakdownRow {
            error_code: r.get(0)?,
            error_message: r.get(1)?,
            count: r.get(2)?,
        })
    })
}
