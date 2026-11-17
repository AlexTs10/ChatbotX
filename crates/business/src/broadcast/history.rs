//! The history and skip parts of `broadcast/recurrence.service.ts`:
//! `setOccurrenceExcluded`, `listRunGroups`, `listOccurrences`, and
//! `groupLedgerCells` / `nextBucketStart`.
//! The recipient `stats` are `broadcastStatsRepository.getBatchStats` /
//! `getStatsByOccurrenceBucket` over `ContactOnBroadcast`.

use chatbotx_core::js::to_iso_string;
use chatbotx_core::utils::recurrence::{
    RECURRENCE_MAX_EXCLUDED_DATES, RECURRENCE_WEEK_STARTS_ON, RecurrenceRule, find_slot_on_date,
    local_date_of, resolve_zoned_instant,
};
use chatbotx_core::utils::timezone::{canonicalize_timezone, is_valid_timezone};
use chatbotx_db::Db;
use chatbotx_db::repo::Json;
use chatbotx_db::repo::broadcast::find_by_id_or_name;
use chatbotx_db::repo::broadcast_recurrence::{
    self as repository, LedgerBucketCell, RecurrencePatch,
};
use chatbotx_db::repo::models::{Broadcast, BroadcastRecurrence};
use chatbotx_db::repo::queries::date_bucket::{Granularity, zoned_bucket_key};
use chatbotx_db::repo::sql::Sql;
use chatbotx_db::rusqlite::Connection;
use serde_json::{Map, Value, json};

use super::is_recurring_template;
use super::recurrence::{next_send_at_of, rule_to_json, stored_rule, upcoming_runs_of};
use crate::audit::AuditService;
use crate::errors::{BusinessError, Result};
use crate::tx;

/// A `scheduled` run older than this is shown as stalled.
pub const BROADCAST_RUN_STALLED_AFTER_MS: i64 = 30 * 60 * 1000;
pub const BROADCAST_RUN_GROUPS_MAX_BUCKETS: i64 = 31;
pub const BROADCAST_OCCURRENCES_MAX_PER_PAGE: i64 = 100;
const SKIP_RESPONSE_UPCOMING_RUNS: usize = 3;
const DAY_MS: i64 = 86_400_000;

fn invalid(message: &str) -> BusinessError {
    BusinessError::validation("recurrence", message)
}

fn iso(ms: i64) -> Value {
    to_iso_string(ms).map_or(Value::Null, Value::String)
}

/// `findSeries`: the template by id or name and its series row.
fn find_series(
    conn: &Connection,
    workspace_id: i64,
    id_or_name: &str,
) -> Result<(Broadcast, BroadcastRecurrence)> {
    let template = find_by_id_or_name(conn, workspace_id, id_or_name)?
        .ok_or_else(|| BusinessError::not_found("Broadcast not found"))?
        .broadcast;
    let recurrence = repository::find_by_broadcast_id(conn, template.id)?
        .ok_or_else(|| BusinessError::not_found("This broadcast does not repeat"))?;
    Ok((template, recurrence))
}

fn rule_of(recurrence: &BroadcastRecurrence) -> Result<RecurrenceRule> {
    stored_rule(&recurrence.rule).ok_or_else(|| invalid("Invalid recurrence"))
}

/// `requestedDates`: distinct, sorted local dates from `dates` and `occurrenceAts`.
fn requested_dates(
    rule: &RecurrenceRule,
    dates: &[String],
    occurrence_ats: &[i64],
) -> Result<Vec<String>> {
    let mut requested: Vec<String> = dates.to_vec();
    requested.extend(
        occurrence_ats
            .iter()
            .map(|at| local_date_of(*at, &rule.timezone)),
    );
    if requested.is_empty() {
        return Err(invalid("Name at least one run to skip or restore"));
    }
    requested.sort();
    requested.dedup();
    Ok(requested)
}

/// `assertSkippable`: a live repeating template, running or paused.
fn assert_skippable(recurrence: &BroadcastRecurrence, template: &Broadcast) -> Result<()> {
    let skippable = template.deleted_at.is_none()
        && is_recurring_template(template)
        && recurrence.ended_at.is_none()
        && matches!(template.status.as_str(), "recurring" | "cancelled");
    if skippable {
        Ok(())
    } else {
        Err(invalid(
            "Only a repeating broadcast that is running or paused can skip a run",
        ))
    }
}

/// `assertDateCanChange`: the date is a run that has not started or been settled.
fn assert_date_can_change(
    recurrence: &BroadcastRecurrence,
    rule: &RecurrenceRule,
    date: &str,
    excluded: bool,
    now: i64,
) -> Result<()> {
    let slot = find_slot_on_date(rule, date, recurrence.occurrence_count)
        .ok()
        .flatten();
    let Some(slot) = slot else {
        // A leftover date that no longer matches a run may always be removed.
        return if excluded {
            Err(invalid("The schedule has no run on that date"))
        } else {
            Ok(())
        };
    };
    let settled = recurrence.accounted_through_at.is_some_and(|at| slot <= at);
    if slot <= now || settled {
        return Err(invalid(
            "That run has already started and can no longer change",
        ));
    }
    Ok(())
}

/// `withExcludedDates`: the rule with dates added or removed, settled dates dropped.
fn with_excluded_dates(
    recurrence: &BroadcastRecurrence,
    rule: &RecurrenceRule,
    dates: &[String],
    excluded: bool,
) -> Result<RecurrenceRule> {
    let settled_date = recurrence
        .accounted_through_at
        .map(|at| local_date_of(at, &rule.timezone));
    let mut next: Vec<String> = rule
        .exclude_dates
        .iter()
        .flatten()
        .filter(|d| !dates.contains(d) && settled_date.as_ref().is_none_or(|s| *d >= s))
        .cloned()
        .collect();
    if excluded {
        next.extend(dates.iter().cloned());
    }
    next.sort();
    next.dedup();
    if next.len() > RECURRENCE_MAX_EXCLUDED_DATES {
        return Err(invalid(&format!(
            "Skip at most {RECURRENCE_MAX_EXCLUDED_DATES} dates"
        )));
    }
    let mut updated = rule.clone();
    updated.exclude_dates = (!next.is_empty()).then_some(next);
    Ok(updated)
}

/// `describeSkipState`: `{ excludeDates, nextSendAt, upcomingRuns }`.
fn describe_skip_state(recurrence: &BroadcastRecurrence, rule: &RecurrenceRule, now: i64) -> Value {
    let rule_json = Json(rule_to_json(rule));
    let next_send_at = next_send_at_of(
        &rule_json,
        recurrence.next_run_at,
        recurrence.occurrence_count,
    );
    let upcoming = if recurrence.next_run_at.is_some() {
        upcoming_runs_of(
            rule,
            recurrence.occurrence_count,
            now,
            SKIP_RESPONSE_UPCOMING_RUNS,
        )
    } else {
        Vec::new()
    };
    json!({
        "excludeDates": rule.exclude_dates.clone().unwrap_or_default(),
        "nextSendAt": next_send_at.map_or(Value::Null, iso),
        "upcomingRuns": upcoming,
    })
}

/// `setOccurrenceExcluded`: skip (`excluded`) or restore runs by local date or instant.
pub async fn set_occurrence_excluded(
    db: &Db,
    audit: &AuditService,
    workspace_id: i64,
    id_or_name: String,
    dates: Vec<String>,
    occurrence_ats: Vec<i64>,
    excluded: bool,
) -> Result<Value> {
    let (state, applied, broadcast_id) = tx::write(db, move |tx| {
        let (_, unlocked) = find_series(tx, workspace_id, &id_or_name)?;
        // The series row first, then its template (the shared lock order).
        let (locked, template) = repository::find_for_materialize(tx, unlocked.id, workspace_id)?
            .ok_or_else(|| BusinessError::not_found("Broadcast not found"))?;
        assert_skippable(&locked, &template)?;
        let rule = rule_of(&locked)?;
        let requested = requested_dates(&rule, &dates, &occurrence_ats)?;
        let current = rule.exclude_dates.clone().unwrap_or_default();
        // A date already in the wanted state is a no-op, not an error.
        let pending: Vec<String> = requested
            .into_iter()
            .filter(|d| current.contains(d) != excluded)
            .collect();
        let now = repository::select_db_now(tx)?;
        if pending.is_empty() {
            return Ok((
                describe_skip_state(&locked, &rule, now),
                pending,
                unlocked.broadcast_id,
            ));
        }
        for date in &pending {
            assert_date_can_change(&locked, &rule, date, excluded, now)?;
        }
        let updated = with_excluded_dates(&locked, &rule, &pending, excluded)?;
        let patch = RecurrencePatch {
            rule: Some(Json(rule_to_json(&updated))),
            ..RecurrencePatch::default()
        };
        repository::update(tx, locked.id, &patch, None)?;
        Ok((
            describe_skip_state(&locked, &updated, now),
            pending,
            unlocked.broadcast_id,
        ))
    })
    .await?;
    if !applied.is_empty() {
        let verb = if excluded { "skipped" } else { "restored" };
        audit
            .audit(
                "update",
                &format!(
                    "{verb} recurring broadcast runs on {} (#{broadcast_id})",
                    applied.join(", ")
                ),
            )
            .await;
    }
    Ok(state)
}

/// `nextBucketStart`: the first local day of the bucket after `bucket`.
pub fn next_bucket_start(bucket: &str, granularity: Granularity) -> String {
    let start = resolve_zoned_instant(bucket, "00:00", "UTC");
    let next = match granularity {
        Granularity::Day => start + DAY_MS,
        Granularity::Week => start + 7 * DAY_MS,
        Granularity::Month => {
            let (year, month) = (&bucket[0..4], &bucket[5..7]);
            let (year, month): (i32, u32) =
                (year.parse().unwrap_or(1970), month.parse().unwrap_or(1));
            let (y, m) = if month == 12 {
                (year + 1, 1)
            } else {
                (year, month + 1)
            };
            return format!("{y:04}-{m:02}-01");
        }
    };
    to_iso_string(next).map_or_else(String::new, |s| s[..10].to_owned())
}

fn add_count(counts: &mut Map<String, Value>, key: &str, amount: i64) {
    let total = counts.get(key).and_then(Value::as_i64).unwrap_or(0) + amount;
    counts.insert(key.to_owned(), json!(total));
}

/// `groupLedgerCells`: one group per bucket, with its boundaries and counts.
pub fn group_ledger_cells(
    buckets: &[String],
    cells: &[LedgerBucketCell],
    granularity: Granularity,
    timezone: &str,
) -> Vec<Value> {
    buckets
        .iter()
        .map(|bucket| {
            let period_end = next_bucket_start(bucket, granularity);
            let (mut slots, mut created, mut skipped, mut missed) = (0, 0, 0, 0);
            let (mut reasons, mut run_states) = (Map::new(), Map::new());
            for cell in cells.iter().filter(|c| &c.bucket == bucket) {
                slots += cell.slots;
                match cell.outcome.as_str() {
                    "created" => {
                        created += cell.slots;
                        if let Some(status) = &cell.run_status {
                            add_count(&mut run_states, status, cell.slots);
                        }
                    }
                    "skipped" => skipped += cell.slots,
                    _ => missed += cell.slots,
                }
                if let Some(reason) = &cell.reason {
                    add_count(&mut reasons, reason, cell.slots);
                }
            }
            json!({
                "periodStart": bucket,
                "periodEnd": period_end,
                "periodStartAt": iso(resolve_zoned_instant(bucket, "00:00", timezone)),
                "periodEndAt": iso(resolve_zoned_instant(&period_end, "00:00", timezone)),
                "slotCount": slots,
                "createdCount": created,
                "skippedCount": skipped,
                "missedCount": missed,
                "reasonCounts": reasons,
                "runStateCounts": run_states,
            })
        })
        .collect()
}

const RECIPIENT_COUNTS: &str = "sum(t.\"deliveredAt\" IS NOT NULL), sum(t.\"seenAt\" IS NOT NULL), \
     sum(t.\"clickedAt\" IS NOT NULL), sum(t.\"failedAt\" IS NOT NULL)";

/// `emptyStats` / `toStats` of `(delivered, seen, clicked, failed)`.
fn to_stats((delivered, seen, clicked, failed): (i64, i64, i64, i64)) -> Value {
    json!({
        "message:sent": delivered + failed,
        "message:delivered": delivered,
        "message:seen": seen,
        "flow:clicked": clicked,
        "message:failed": failed,
    })
}

fn counts_at(
    row: &chatbotx_db::rusqlite::Row<'_>,
    from: usize,
) -> chatbotx_db::rusqlite::Result<(i64, i64, i64, i64)> {
    Ok((
        row.get(from)?,
        row.get(from + 1)?,
        row.get(from + 2)?,
        row.get(from + 3)?,
    ))
}

/// `getBatchStats`: recipient counts per run (empty stats for a run without recipients).
pub fn batch_stats(
    conn: &Connection,
    workspace_id: i64,
    broadcast_ids: &[i64],
) -> Result<Map<String, Value>> {
    let mut out: Map<String, Value> = broadcast_ids
        .iter()
        .map(|id| (id.to_string(), to_stats((0, 0, 0, 0))))
        .collect();
    if broadcast_ids.is_empty() {
        return Ok(out);
    }
    let mut q = Sql::raw(format!(
        "SELECT t.\"broadcastId\", {RECIPIENT_COUNTS} FROM \"ContactOnBroadcast\" AS t \
         INNER JOIN \"Broadcast\" AS b ON b.\"id\" = t.\"broadcastId\" \
         WHERE t.\"broadcastId\" IN (SELECT value FROM json_each("
    ));
    q.push_bind(serde_json::to_string(broadcast_ids).unwrap_or_default());
    q.push(")) AND b.\"workspaceId\" = ");
    q.push_bind(workspace_id);
    q.push(" GROUP BY t.\"broadcastId\"");
    let rows = q.query_map(conn, |row| Ok((row.get::<_, i64>(0)?, counts_at(row, 1)?)))?;
    for (id, counts) in rows {
        out.insert(id.to_string(), to_stats(counts));
    }
    Ok(out)
}

/// `getStatsByOccurrenceBucket`: recipient counts of the runs per bucket of `occurrenceAt`.
pub fn stats_by_occurrence_bucket(
    conn: &Connection,
    workspace_id: i64,
    parent_broadcast_id: i64,
    granularity: Granularity,
    timezone: &str,
    buckets: &[String],
) -> Result<Map<String, Value>> {
    let mut out: Map<String, Value> = buckets
        .iter()
        .map(|b| (b.clone(), to_stats((0, 0, 0, 0))))
        .collect();
    if buckets.is_empty() {
        return Ok(out);
    }
    let bucket = zoned_bucket_key("b.\"occurrenceAt\"", granularity, timezone);
    let mut q = Sql::raw("SELECT ");
    q.append(bucket.clone());
    q.push(&format!(
        ", {RECIPIENT_COUNTS} FROM \"ContactOnBroadcast\" AS t \
         INNER JOIN \"Broadcast\" AS b ON b.\"id\" = t.\"broadcastId\" WHERE b.\"workspaceId\" = "
    ));
    q.push_bind(workspace_id);
    q.push(" AND b.\"parentBroadcastId\" = ");
    q.push_bind(parent_broadcast_id);
    q.push(" AND ");
    q.append(bucket);
    q.push(" IN (SELECT value FROM json_each(");
    q.push_bind(chatbotx_db::repo::text_list(buckets));
    q.push(")) GROUP BY 1");
    let rows = q.query_map(conn, |row| {
        Ok((row.get::<_, String>(0)?, counts_at(row, 1)?))
    })?;
    for (key, counts) in rows {
        out.insert(key, to_stats(counts));
    }
    Ok(out)
}

/// `OccurrenceBucketGranularity` of `groupBy`.
pub fn granularity_of(name: &str) -> Option<Granularity> {
    match name {
        "day" => Some(Granularity::Day),
        "week" => Some(Granularity::Week),
        "month" => Some(Granularity::Month),
        _ => None,
    }
}

/// `listRunGroups`: slots grouped by local day / ISO week / month, newest first.
pub async fn list_run_groups(
    db: &Db,
    workspace_id: i64,
    id_or_name: String,
    granularity: Granularity,
    timezone: Option<String>,
    limit: i64,
    cursor: Option<String>,
) -> Result<Value> {
    tx::read(db, move |conn| {
        let (template, recurrence) = find_series(conn, workspace_id, &id_or_name)?;
        let rule = rule_of(&recurrence)?;
        let timezone = timezone
            .filter(|t| !t.is_empty())
            .map(|t| canonicalize_timezone(&t))
            .filter(|t| is_valid_timezone(t))
            .unwrap_or(rule.timezone);
        let limit = limit.min(BROADCAST_RUN_GROUPS_MAX_BUCKETS);
        // One extra key says whether another page exists.
        let fetched = repository::list_ledger_bucket_keys(
            conn,
            recurrence.id,
            granularity,
            &timezone,
            limit + 1,
            cursor.as_deref(),
        )?;
        let buckets: Vec<String> = fetched.iter().take(limit as usize).cloned().collect();
        let cells = repository::list_ledger_bucket_cells(
            conn,
            recurrence.id,
            granularity,
            &timezone,
            &buckets,
        )?;
        let stats = stats_by_occurrence_bucket(
            conn,
            workspace_id,
            template.id,
            granularity,
            &timezone,
            &buckets,
        )?;
        let total_slots = repository::count_ledger_slots(conn, recurrence.id)?;
        let next_cursor = if fetched.len() as i64 > limit {
            buckets.last().map_or(Value::Null, |b| json!(b))
        } else {
            Value::Null
        };
        Ok(json!({
            "data": group_ledger_cells(&buckets, &cells, granularity, &timezone)
                .into_iter()
                .map(|mut group| {
                    let key = group["periodStart"].as_str().unwrap_or_default().to_owned();
                    if let Some(found) = stats.get(&key) {
                        group["stats"] = found.clone();
                    }
                    group
                })
                .collect::<Vec<_>>(),
            "nextCursor": next_cursor,
            "totalSlots": total_slots,
            "totalRuns": recurrence.occurrence_count,
            "timezone": timezone,
            "weekStartsOn": RECURRENCE_WEEK_STARTS_ON,
        }))
    })
    .await
}

/// `decodeOccurrenceCursor`: `<epoch ms>:<ledger row id>`.
fn decode_occurrence_cursor(cursor: Option<&str>) -> Result<Option<(i64, i64)>> {
    let Some(cursor) = cursor.filter(|c| !c.is_empty()) else {
        return Ok(None);
    };
    let bad = || BusinessError::validation("cursor", "Invalid cursor");
    let (at, id) = cursor.split_once(':').ok_or_else(bad)?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(at) || !digits(id) {
        return Err(bad());
    }
    Ok(Some((
        at.parse().map_err(|_| bad())?,
        id.parse().map_err(|_| bad())?,
    )))
}

/// `listOccurrences`: one page of slots, newest first, each with its run.
pub async fn list_occurrences(
    db: &Db,
    workspace_id: i64,
    id_or_name: String,
    from: Option<i64>,
    to: Option<i64>,
    cursor: Option<String>,
    limit: i64,
) -> Result<Value> {
    tx::read(db, move |conn| {
        let (_, recurrence) = find_series(conn, workspace_id, &id_or_name)?;
        let limit = limit.min(BROADCAST_OCCURRENCES_MAX_PER_PAGE);
        let cursor = decode_occurrence_cursor(cursor.as_deref())?;
        let rows = repository::list_ledger_rows(conn, recurrence.id, from, to, cursor, limit + 1)?;
        let page = &rows[..rows.len().min(limit as usize)];
        let run_ids: Vec<i64> = page
            .iter()
            .filter_map(|r| r.run.as_ref().map(|run| run.id))
            .collect();
        let stats = batch_stats(conn, workspace_id, &run_ids)?;
        let now = chatbotx_db::repo::now_ms();
        let data: Vec<Value> = page
            .iter()
            .map(|row| {
                let o = &row.occurrence;
                let run = row.run.as_ref().map_or(Value::Null, |run| {
                    let mut out = json!({
                        "id": run.id.to_string(),
                        "status": run.status,
                        "contactCount": run.contact_count,
                        "isStalled": run.status == "scheduled"
                            && now - o.occurrence_at > BROADCAST_RUN_STALLED_AFTER_MS,
                    });
                    if let Some(found) = stats.get(&run.id.to_string()) {
                        out["stats"] = found.clone();
                    }
                    out
                });
                json!({
                    "localSlot": o.local_slot,
                    "occurrenceAt": iso(o.occurrence_at),
                    "outcome": o.outcome,
                    "reason": o.reason,
                    "latenessMs": (o.materialized_at - o.occurrence_at).max(0),
                    "supersededCount": o.superseded_count,
                    "run": run,
                })
            })
            .collect();
        let next_cursor = match page.last() {
            Some(last) if rows.len() > page.len() => json!(format!(
                "{}:{}",
                last.occurrence.occurrence_at, last.occurrence.id
            )),
            _ => Value::Null,
        };
        Ok(json!({ "data": data, "nextCursor": next_cursor }))
    })
    .await
}
