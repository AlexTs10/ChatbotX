//! Port of `repositories/google-ads-conversion-event/repository.ts` (P3.2b).
//!
//! Instants are unix ms. `value` is the exact decimal text of the `numeric` column.

use rusqlite::types::Value;
use rusqlite::{Connection, Row};

use super::models::GoogleAdsConversionEvent;
use super::patch::{self, Patch};
use super::sql::{Sql, opt_int, opt_text};
use super::{Error, Model, Result, iso, new_id, now_ms};

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

const TABLE: &str = "\"GoogleAdsConversionEvent\"";

fn cols() -> String {
    GoogleAdsConversionEvent::columns_sql(None)
}

fn eq(column: &str, value: impl Into<Value>) -> Option<Sql> {
    Some(Sql::bind(format!("\"{column}\" = "), value))
}

fn opt_json(v: Option<&serde_json::Value>) -> Value {
    v.map_or(Value::Null, patch::json)
}

/// `UPDATE ... SET <set> WHERE <where> RETURNING *` (at most one row).
fn update_returning(
    conn: &Connection,
    set: Sql,
    filter: Sql,
) -> Result<Option<GoogleAdsConversionEvent>> {
    let mut q = Sql::raw(format!("UPDATE {TABLE} SET "));
    q.append(set);
    q.push(" WHERE ");
    q.append(filter);
    q.push(&format!(" RETURNING {}", cols()));
    q.model(conn)
}

/// A `"a" = ?, ...` list of `(column, value)` pairs.
fn set_list(pairs: Vec<(&str, Value)>) -> Sql {
    let mut out = Sql::default();
    for (n, (col, value)) in pairs.into_iter().enumerate() {
        if n > 0 {
            out.push(", ");
        }
        out.push(&format!("\"{col}\" = "));
        out.push_bind(value);
    }
    out
}

fn id_ws(id: i64, workspace_id: i64) -> [Option<Sql>; 2] {
    [eq("id", id), eq("workspaceId", workspace_id)]
}

/// `windowFilters`: the workspace always, the rest when given (bounds inclusive). `p` is
/// the table alias prefix (`""`, `"g."`, ..).
fn window_filters(
    p: &str,
    workspace_id: i64,
    channel: Option<&str>,
    conversion_action_id: Option<&str>,
    since: Option<i64>,
    until: Option<i64>,
) -> Vec<Option<Sql>> {
    vec![
        Some(Sql::bind(format!("{p}\"workspaceId\" = "), workspace_id)),
        channel.map(|c| Sql::bind(format!("{p}\"channel\" = "), c.to_owned())),
        conversion_action_id
            .map(|a| Sql::bind(format!("{p}\"conversionActionId\" = "), a.to_owned())),
        since.map(|v| Sql::bind(format!("{p}\"occurredAt\" >= "), v)),
        until.map(|v| Sql::bind(format!("{p}\"occurredAt\" <= "), v)),
    ]
}

fn stats_where(p: &str, f: &StatsFilters) -> Vec<Option<Sql>> {
    window_filters(
        p,
        f.workspace_id,
        f.channel.as_deref(),
        f.conversion_action_id.as_deref(),
        Some(f.since),
        Some(f.until),
    )
}

const STATUSES: [&str; 7] = [
    "pending",
    "sending",
    "sent",
    "processed",
    "failed",
    "skipped_no_account",
    "skipped_expired",
];

/// `statusCountColumns` (`count(*) FILTER (WHERE status = ..)` as `SUM(CASE ..)`).
fn status_count_sql() -> String {
    STATUSES
        .iter()
        .map(|s| format!("COALESCE(SUM(\"status\" = '{s}'), 0)"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn status_counts(row: &Row<'_>, first: usize) -> rusqlite::Result<StatusCounts> {
    Ok(StatusCounts {
        pending: row.get(first)?,
        sending: row.get(first + 1)?,
        sent: row.get(first + 2)?,
        processed: row.get(first + 3)?,
        failed: row.get(first + 4)?,
        skipped_no_account: row.get(first + 5)?,
        skipped_expired: row.get(first + 6)?,
    })
}

/// `insertIgnoreDuplicate`: `values` must carry `options` (the TS type requires it).
pub fn insert_ignore_duplicate(
    conn: &Connection,
    values: &Patch,
) -> Result<Option<GoogleAdsConversionEvent>> {
    if !values.iter().any(|(c, _)| *c == "options") {
        return Err(Error::msg(
            "insertIgnoreDuplicate: the options snapshot is required",
        ));
    }
    let mut all: Patch = Vec::with_capacity(values.len() + 1);
    if !values.iter().any(|(c, _)| *c == "id") {
        all.push(("id", Value::Integer(new_id())));
    }
    all.extend(values.iter().cloned());
    let mut q = Sql::raw(format!("INSERT INTO {TABLE} "));
    q.append(patch::insert_clause(
        GoogleAdsConversionEvent::COLUMNS,
        &all,
    )?);
    q.push(&format!(
        " ON CONFLICT (\"workspaceId\", \"transactionId\") DO NOTHING RETURNING {}",
        cols()
    ));
    q.model(conn)
}

/// `findByTransactionId`.
pub fn find_by_transaction_id(
    conn: &Connection,
    workspace_id: i64,
    transaction_id: &str,
) -> Result<Option<GoogleAdsConversionEvent>> {
    super::find_one(
        conn,
        "\"workspaceId\" = ?1 AND \"transactionId\" = ?2",
        &[&workspace_id, &transaction_id],
    )
}

/// `findWorkspaceEvent`.
pub fn find_workspace_event(
    conn: &Connection,
    event: EventRef,
) -> Result<Option<GoogleAdsConversionEvent>> {
    super::find_one(
        conn,
        "\"id\" = ?1 AND \"workspaceId\" = ?2",
        &[&event.id, &event.workspace_id],
    )
}

/// `claimForSending`.
pub fn claim_for_sending(
    conn: &Connection,
    input: &ClaimInput,
) -> Result<Option<GoogleAdsConversionEvent>> {
    let [a, b] = id_ws(input.id, input.workspace_id);
    update_returning(
        conn,
        set_list(vec![
            ("status", Value::from("sending".to_owned())),
            ("claimToken", Value::from(input.claim_token.clone())),
            ("claimedAt", Value::Integer(now_ms())),
            ("updatedAt", Value::Integer(now_ms())),
        ]),
        Sql::and([
            a,
            b,
            eq("status", "pending".to_owned()),
            eq("attempt", input.attempt),
        ]),
    )
}

/// `finishSending`.
pub fn finish_sending(
    conn: &Connection,
    input: &FinishSendingInput,
) -> Result<Option<GoogleAdsConversionEvent>> {
    let [a, b] = id_ws(input.id, input.workspace_id);
    update_returning(
        conn,
        set_list(vec![
            ("status", Value::from(input.to.clone())),
            ("claimToken", Value::Null),
            ("claimedAt", Value::Null),
            ("requestId", opt_text(input.request_id.as_deref())),
            ("sentAt", opt_int(input.sent_at)),
            ("error", opt_text(input.error.as_deref())),
            ("failureStage", opt_text(input.failure_stage.as_deref())),
            (
                "processingStatus",
                opt_text(input.processing_status.as_deref()),
            ),
            (
                "nextProcessingCheckAt",
                opt_int(input.next_processing_check_at),
            ),
            ("updatedAt", Value::Integer(now_ms())),
        ]),
        Sql::and([
            a,
            b,
            eq("status", "sending".to_owned()),
            eq("claimToken", input.claim_token.clone()),
        ]),
    )
}

fn lease_fence(input: &ClaimInput) -> Sql {
    let [a, b] = id_ws(input.id, input.workspace_id);
    Sql::and([
        a,
        b,
        eq("status", "sending".to_owned()),
        eq("claimToken", input.claim_token.clone()),
        eq("attempt", input.attempt),
    ])
}

/// `markSendAttempted` (`at` = unix ms): the first stamp wins.
pub fn mark_send_attempted(
    conn: &Connection,
    input: &ClaimInput,
    at: i64,
) -> Result<Option<GoogleAdsConversionEvent>> {
    let mut set = Sql::raw(
        "\"processingDetail\" = (SELECT json_group_object(key, value) FROM (\
         SELECT key, value FROM json_each(json_object('sendAttemptedAt', ?)) \
         WHERE key NOT IN (SELECT key FROM json_each(COALESCE(\"processingDetail\", '{}'))) \
         UNION ALL SELECT key, value FROM json_each(COALESCE(\"processingDetail\", '{}'))))",
    );
    set.params.push(Value::from(iso(at)));
    set.push(", \"updatedAt\" = ");
    set.push_bind(now_ms());
    update_returning(conn, set, lease_fence(input))
}

/// `finishSendingProcessed`.
pub fn finish_sending_processed(
    conn: &Connection,
    input: &FinishSendingProcessedInput,
) -> Result<Option<GoogleAdsConversionEvent>> {
    let now = now_ms();
    update_returning(
        conn,
        set_list(vec![
            ("status", Value::from("processed".to_owned())),
            ("processingStatus", Value::from("success".to_owned())),
            ("requestId", Value::from(input.request_id.clone())),
            ("sentAt", Value::Integer(input.sent_at)),
            ("processingCheckedAt", Value::Integer(now)),
            ("processingAttempts", Value::Integer(1)),
            ("processingDetail", patch::json(&input.processing_detail)),
            ("claimToken", Value::Null),
            ("claimedAt", Value::Null),
            ("error", Value::Null),
            ("failureStage", Value::Null),
            ("nextProcessingCheckAt", Value::Null),
            ("updatedAt", Value::Integer(now)),
        ]),
        lease_fence(&ClaimInput {
            id: input.id,
            workspace_id: input.workspace_id,
            claim_token: input.claim_token.clone(),
            attempt: input.attempt,
        }),
    )
}

/// `releaseClaim`.
pub fn release_claim(
    conn: &Connection,
    event: EventRef,
    claim_token: &str,
    next_attempt: i64,
) -> Result<Option<GoogleAdsConversionEvent>> {
    let [a, b] = id_ws(event.id, event.workspace_id);
    update_returning(
        conn,
        set_list(vec![
            ("status", Value::from("pending".to_owned())),
            ("attempt", Value::Integer(next_attempt)),
            ("claimToken", Value::Null),
            ("claimedAt", Value::Null),
            ("updatedAt", Value::Integer(now_ms())),
        ]),
        Sql::and([
            a,
            b,
            eq("status", "sending".to_owned()),
            eq("claimToken", claim_token.to_owned()),
        ]),
    )
}

/// `redrive`.
pub fn redrive(
    conn: &Connection,
    input: &RedriveInput,
) -> Result<Option<GoogleAdsConversionEvent>> {
    let mut set = set_list(vec![
        ("status", Value::from("pending".to_owned())),
        ("attempt", Value::Integer(input.expected_attempt + 1)),
        ("claimToken", Value::Null),
        ("claimedAt", Value::Null),
        ("failureStage", Value::Null),
        ("error", Value::Null),
        ("requestId", Value::Null),
        ("sentAt", Value::Null),
        ("processingStatus", Value::Null),
        ("processingCheckedAt", Value::Null),
        ("processingAttempts", Value::Integer(0)),
        ("nextProcessingCheckAt", Value::Null),
        ("updatedAt", Value::Integer(now_ms())),
    ]);
    // Only the "an upload may already have left" stamp outlives a redrive.
    set.push(
        ", \"processingDetail\" = CASE WHEN \"processingDetail\" ->> '$.sendAttemptedAt' IS NOT NULL \
         THEN json_object('sendAttemptedAt', \"processingDetail\" ->> '$.sendAttemptedAt') ELSE NULL END",
    );
    let statuses = if input.from_statuses.is_empty() {
        Some(Sql::raw("0"))
    } else {
        let marks = vec!["?"; input.from_statuses.len()].join(", ");
        Some(Sql::new(
            format!("\"status\" IN ({marks})"),
            input
                .from_statuses
                .iter()
                .map(|s| Value::from(s.clone()))
                .collect(),
        ))
    };
    let claim = match &input.expected_claim_token {
        None => None,
        Some(None) => Some(Sql::raw("\"claimToken\" IS NULL")),
        Some(Some(token)) => eq("claimToken", token.clone()),
    };
    let [a, b] = id_ws(input.id, input.workspace_id);
    update_returning(
        conn,
        set,
        Sql::and([
            a,
            b,
            statuses,
            eq("attempt", input.expected_attempt),
            input
                .expected_request_id
                .as_ref()
                .and_then(|r| eq("requestId", r.clone())),
            claim,
        ]),
    )
}

/// `applyProcessingResult`.
pub fn apply_processing_result(
    conn: &Connection,
    input: &ApplyProcessingResultInput,
) -> Result<Option<GoogleAdsConversionEvent>> {
    let now = now_ms();
    let [a, b] = id_ws(input.id, input.workspace_id);
    update_returning(
        conn,
        set_list(vec![
            ("status", Value::from(input.to.clone())),
            (
                "processingStatus",
                Value::from(input.processing_status.clone()),
            ),
            (
                "processingDetail",
                opt_json(input.processing_detail.as_ref()),
            ),
            ("processingCheckedAt", Value::Integer(now)),
            (
                "processingAttempts",
                Value::Integer(input.processing_attempts),
            ),
            (
                "nextProcessingCheckAt",
                opt_int(input.next_processing_check_at),
            ),
            ("failureStage", opt_text(input.failure_stage.as_deref())),
            ("error", opt_text(input.error.as_deref())),
            ("updatedAt", Value::Integer(now)),
        ]),
        Sql::and([
            a,
            b,
            eq("status", "sent".to_owned()),
            eq("attempt", input.expected_attempt),
            Some(match &input.expected_request_id {
                None => Sql::raw("\"requestId\" IS NULL"),
                Some(r) => Sql::bind("\"requestId\" = ", r.clone()),
            }),
        ]),
    )
}

/// `listByWorkspace`.
pub fn list_by_workspace(conn: &Connection, input: &ListInput) -> Result<ListResult> {
    let per_page = input.per_page.min(GOOGLE_ADS_EVENTS_MAX_PER_PAGE);
    let mut parts = window_filters(
        "",
        input.workspace_id,
        input.channel.as_deref(),
        input.conversion_action_id.as_deref(),
        input.since,
        input.until,
    );
    parts.push(input.status.as_ref().and_then(|s| eq("status", s.clone())));
    let filter = Sql::and(parts);
    let mut q = Sql::raw(format!("SELECT {} FROM {TABLE} WHERE ", cols()));
    q.append(filter.clone());
    q.push(" ORDER BY \"occurredAt\" DESC, \"id\" DESC LIMIT ");
    q.push_bind(per_page);
    q.push(" OFFSET ");
    q.push_bind((input.page - 1) * per_page);
    let rows = q.models(conn)?;
    let mut c = Sql::raw(format!("SELECT count(*) FROM {TABLE} WHERE "));
    c.append(filter);
    let total = c
        .query_map(conn, |r| r.get::<_, i64>(0))?
        .into_iter()
        .next()
        .unwrap_or(0);
    Ok(ListResult { rows, total })
}

/// `statsByDayAndChannel`.
pub fn stats_by_day_and_channel(
    conn: &Connection,
    filters: &StatsFilters,
    timezone: &str,
) -> Result<Vec<DayChannelStatsRow>> {
    let mut q = Sql::raw("SELECT ");
    q.append(super::queries::date_bucket::zoned_date_key(
        "\"occurredAt\"",
        timezone,
    ));
    q.push(&format!(
        " AS \"date\", \"channel\", {}, \
         COALESCE(SUM(\"status\" = 'failed' AND \"failureStage\" = 'delivery'), 0), \
         COALESCE(SUM(\"status\" = 'failed' AND \"failureStage\" = 'processing'), 0), \
         COALESCE(SUM(\"status\" = 'failed' AND \"failureStage\" = 'timeout'), 0), \
         COALESCE(SUM(\"status\" = 'failed' AND \"failureStage\" IS NULL), 0) \
         FROM {TABLE} WHERE ",
        status_count_sql()
    ));
    q.append(Sql::and(stats_where("", filters)));
    q.push(" GROUP BY 1, \"channel\"");
    q.query_map(conn, |r| {
        Ok(DayChannelStatsRow {
            date: r.get(0)?,
            channel: r.get(1)?,
            counts: status_counts(r, 2)?,
            failed_delivery: r.get(9)?,
            failed_processing: r.get(10)?,
            failed_timeout: r.get(11)?,
            failed_unknown: r.get(12)?,
        })
    })
}

/// `statsByAction`: busiest first, `limit + 1` rows.
pub fn stats_by_action(
    conn: &Connection,
    filters: &StatsFilters,
    limit: i64,
) -> Result<Vec<ActionStatsRow>> {
    // `latestOf`: the newest snapshot (occurredAt DESC, id DESC) of the group's rows.
    let latest = |column: &str| {
        let mut s = Sql::raw(format!(
            "(SELECT l.\"{column}\" FROM {TABLE} AS l WHERE l.\"conversionActionId\" = g.\"conversionActionId\" AND "
        ));
        s.append(Sql::and(stats_where("l.", filters)));
        s.push(" ORDER BY l.\"occurredAt\" DESC, l.\"id\" DESC LIMIT 1)");
        s
    };
    let mut q = Sql::raw("SELECT g.\"conversionActionId\", ");
    q.append(latest("conversionActionName"));
    q.push(", ");
    q.append(latest("conversionActionCategory"));
    q.push(&format!(
        ", {} FROM {TABLE} AS g WHERE ",
        status_count_sql().replace("\"status\"", "g.\"status\"")
    ));
    q.append(Sql::and(stats_where("g.", filters)));
    q.push(" GROUP BY g.\"conversionActionId\" ORDER BY count(*) DESC, g.\"conversionActionId\" ASC LIMIT ");
    q.push_bind(limit + 1);
    q.query_map(conn, |r| {
        Ok(ActionStatsRow {
            conversion_action_id: r.get(0)?,
            name: r.get(1)?,
            category: r.get(2)?,
            counts: status_counts(r, 3)?,
        })
    })
}

/// `confirmedValueByCurrency`: the exact decimal sum per currency (`numeric::text`).
pub fn confirmed_value_by_currency(
    conn: &Connection,
    filters: &StatsFilters,
) -> Result<Vec<ConfirmedValueRow>> {
    let mut parts = stats_where("", filters);
    parts.push(eq("status", "processed".to_owned()));
    parts.push(Some(Sql::raw("\"value\" IS NOT NULL")));
    let mut q = Sql::raw(format!(
        "SELECT \"currency\", \"value\" FROM {TABLE} WHERE "
    ));
    q.append(Sql::and(parts));
    q.push(" ORDER BY \"currency\" ASC");
    let rows: Vec<(Option<String>, String)> = q.query_map(conn, |r| Ok((r.get(0)?, r.get(1)?)))?;
    let mut groups: Vec<(Option<String>, Vec<String>)> = Vec::new();
    for (currency, value) in rows {
        match groups.last_mut() {
            Some((c, values)) if *c == currency => values.push(value),
            _ => groups.push((currency, vec![value])),
        }
    }
    groups
        .into_iter()
        .map(|(currency, values)| {
            let value = super::ads_conversion_event::decimal_sum(values.iter().map(String::as_str))
                .ok_or_else(|| Error::msg("confirmedValueByCurrency: not a decimal value"))?;
            Ok(ConfirmedValueRow {
                currency,
                value,
                count: i64::try_from(values.len()).unwrap_or(i64::MAX),
            })
        })
        .collect()
}

/// `existsForWorkspace`.
pub fn exists_for_workspace(conn: &Connection, workspace_id: i64) -> Result<bool> {
    Ok(conn.query_row(
        &format!("SELECT EXISTS (SELECT 1 FROM {TABLE} WHERE \"workspaceId\" = ?1)"),
        [workspace_id],
        |r| r.get::<_, bool>(0),
    )?)
}

/// `listDueForProcessingCheck`.
pub fn list_due_for_processing_check(
    conn: &Connection,
    now: i64,
    limit: i64,
) -> Result<Vec<GoogleAdsConversionEvent>> {
    super::find_many(
        conn,
        "WHERE \"status\" = 'sent' AND \"uploadMethod\" = 'dataManager' \
         AND \"nextProcessingCheckAt\" IS NOT NULL AND \"nextProcessingCheckAt\" <= ?1 \
         ORDER BY \"nextProcessingCheckAt\" ASC, \"id\" ASC LIMIT ?2",
        &[&now, &limit],
    )
}

/// `touchStranded`.
pub fn touch_stranded(
    conn: &Connection,
    event: EventRef,
    status: &str,
    attempt: i64,
) -> Result<()> {
    conn.execute(
        &format!(
            "UPDATE {TABLE} SET \"updatedAt\" = ?1 WHERE \"id\" = ?2 AND \"workspaceId\" = ?3 \
             AND \"status\" = ?4 AND \"attempt\" = ?5"
        ),
        rusqlite::params![now_ms(), event.id, event.workspace_id, status, attempt],
    )?;
    Ok(())
}

/// `listStranded`: stale `sending` rows, then stale `pending` rows (each window limited).
pub fn list_stranded(
    conn: &Connection,
    input: &StrandedInput,
) -> Result<Vec<GoogleAdsConversionEvent>> {
    let mut rows: Vec<GoogleAdsConversionEvent> = super::find_many(
        conn,
        "WHERE \"status\" = 'sending' AND \"claimedAt\" < ?1 ORDER BY \"claimedAt\" ASC LIMIT ?2",
        &[&input.sending_older_than, &input.limit],
    )?;
    rows.extend(super::find_many::<GoogleAdsConversionEvent>(
        conn,
        "WHERE \"status\" = 'pending' AND \"updatedAt\" < ?1 AND \"googleClickReceivedAt\" < ?2 \
         ORDER BY \"updatedAt\" ASC LIMIT ?3",
        &[
            &input.pending_older_than,
            &input.six_hour_gate_before,
            &input.limit,
        ],
    )?);
    Ok(rows)
}
