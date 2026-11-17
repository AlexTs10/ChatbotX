//! `contactAnalyticsService` / `contactStatsRepository`, the contact parts
//! of `macAnalyticsService`, and `inboxService.listAnalyticsFilterOptions`.

use std::collections::HashMap;

use chatbotx_db::repo;
use chatbotx_db::rusqlite::types::Value as Sql;
use chatbotx_db::rusqlite::{Connection, params_from_iter};

use super::{CountPoint, Filter, TimeRange};

/// `contact_created` minus `contact_deleted`.
const BALANCE: &str =
    r#"(SUM("eventType" = 'contact_created') - SUM("eventType" = 'contact_deleted'))"#;

fn count(conn: &Connection, sql: &str, binds: &[Sql]) -> repo::Result<i64> {
    Ok(conn
        .query_row(sql, params_from_iter(binds.iter()), |row| {
            row.get::<_, Option<i64>>(0)
        })?
        .unwrap_or(0))
}

/// `(key, value)` rows.
fn keyed(conn: &Connection, sql: &str, binds: &[Sql]) -> repo::Result<HashMap<String, i64>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt
        .query_map(params_from_iter(binds.iter()), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?
        .collect::<Result<HashMap<_, _>, _>>()?;
    Ok(rows)
}

/// `getContactsCount`: the current `InboxContactStat` total of the
/// workspace's inboxes (`from` / `to` do not narrow it).
pub fn contacts_count(conn: &Connection, workspace_id: i64, filter: &Filter) -> repo::Result<i64> {
    let mut binds = vec![Sql::Integer(workspace_id)];
    let narrow = filter.sql(r#"i."channel""#, r#"ics."inboxId""#, &mut binds);
    count(
        conn,
        &format!(
            r#"SELECT COALESCE(SUM(ics."totalContacts"), 0) FROM "InboxContactStat" ics
               INNER JOIN "Inbox" i ON i."id" = ics."inboxId"
               WHERE i."workspaceId" = ?{narrow}"#
        ),
        &binds,
    )
}

/// Distinct contacts with one event type in the range
/// (`getNewContactsCount` / `getBlockedContactsCount`).
pub fn distinct_contacts_count(
    conn: &Connection,
    range: &TimeRange,
    event_type: &str,
) -> repo::Result<i64> {
    let Some((from, to)) = range.bounds() else {
        return Ok(0);
    };
    let mut binds = vec![Sql::Integer(range.workspace_id)];
    let narrow = range.filter.event_sql(&mut binds);
    binds.extend([
        Sql::Integer(from),
        Sql::Integer(to),
        Sql::Text(event_type.into()),
    ]);
    count(
        conn,
        &format!(
            r#"SELECT COUNT(DISTINCT "contactId") FROM "AnalyticsContactEvent"
               WHERE "workspaceId" = ?{narrow} AND "occurredAt" >= ? AND "occurredAt" <= ?
                 AND "eventType" = ?"#
        ),
        &binds,
    )
}

/// Distinct contacts per local bucket with one event type, zero-filled
/// (`getNewContactsPerDay` / `getBlockedContactsPerDay` and the monthly
/// variants past 60 days).
pub fn distinct_contacts_series(
    conn: &Connection,
    range: &TimeRange,
    event_type: &str,
) -> repo::Result<Vec<CountPoint>> {
    let granularity = range.granularity();
    let Some((from, to)) = range.bounds() else {
        return Ok(Vec::new());
    };
    let key = granularity.key_sql(r#""occurredAt""#, range.tz);
    let mut binds = vec![Sql::Integer(range.workspace_id)];
    let narrow = range.filter.event_sql(&mut binds);
    binds.extend([
        Sql::Integer(from),
        Sql::Integer(to),
        Sql::Text(event_type.into()),
    ]);
    let by_key = keyed(
        conn,
        &format!(
            r#"SELECT {key}, COUNT(DISTINCT "contactId") FROM "AnalyticsContactEvent"
               WHERE "workspaceId" = ?{narrow} AND "occurredAt" >= ? AND "occurredAt" <= ?
                 AND "eventType" = ?
               GROUP BY 1"#
        ),
        &binds,
    )?;
    Ok(granularity
        .series(range)
        .into_iter()
        .map(|bucket| CountPoint {
            count: by_key.get(&bucket.key).copied().unwrap_or(0),
            date: bucket.date,
        })
        .collect())
}

/// `getContactCountsPerDay` / `getContactCountsPerMonth`: the running total
/// (created minus deleted) at each local bucket, from the baseline before
/// the first bucket; a bucket without events carries the last total.
pub fn contact_counts_series(
    conn: &Connection,
    range: &TimeRange,
) -> repo::Result<Vec<CountPoint>> {
    let granularity = range.granularity();
    let Some((from, to)) = range.bounds() else {
        return Ok(Vec::new());
    };
    let Some(start) = granularity.start_of(from, range.tz) else {
        return Ok(Vec::new());
    };
    let baseline = total_before(conn, range, start)?;
    let key = granularity.key_sql(r#""occurredAt""#, range.tz);
    let mut binds = vec![Sql::Integer(range.workspace_id)];
    let narrow = range.filter.event_sql(&mut binds);
    binds.extend([Sql::Integer(start), Sql::Integer(to)]);
    let net = keyed(
        conn,
        &format!(
            r#"SELECT {key}, {BALANCE} FROM "AnalyticsContactEvent"
               WHERE "workspaceId" = ?{narrow} AND "occurredAt" >= ? AND "occurredAt" <= ?
                 AND "eventType" IN ('contact_created', 'contact_deleted')
               GROUP BY 1"#
        ),
        &binds,
    )?;
    let mut total = baseline;
    Ok(granularity
        .series(range)
        .into_iter()
        .map(|bucket| {
            total += net.get(&bucket.key).copied().unwrap_or(0);
            CountPoint {
                date: bucket.date,
                count: total,
            }
        })
        .collect())
}

/// `getTotalBefore`: the balance before `start`; under an inbox filter the
/// current per-inbox count minus the balance since `start` (floored at 0),
/// as events before inbox tracking carry no inbox.
fn total_before(conn: &Connection, range: &TimeRange, start: i64) -> repo::Result<i64> {
    let inbox_filtered = !range.filter.inbox_ids.is_empty();
    let mut binds = vec![Sql::Integer(range.workspace_id)];
    let narrow = range.filter.event_sql(&mut binds);
    binds.push(Sql::Integer(start));
    let op = if inbox_filtered { ">=" } else { "<" };
    let net = count(
        conn,
        &format!(
            r#"SELECT {BALANCE} FROM "AnalyticsContactEvent"
               WHERE "workspaceId" = ?{narrow} AND "occurredAt" {op} ?
                 AND "eventType" IN ('contact_created', 'contact_deleted')"#
        ),
        &binds,
    )?;
    if !inbox_filtered {
        return Ok(net);
    }
    Ok((contacts_count(conn, range.workspace_id, &range.filter)? - net).max(0))
}

/// `contactDimensionSchema`.
pub const DIMENSIONS: &[&str] = &["country", "channel", "source", "language"];
/// `contactDimensionMetricSchema`.
pub const METRICS: &[&str] = &["new", "total"];

/// `isSupportedDimensionMetric`: `total` only by channel / source, and only
/// by channel under an inbox filter.
pub fn is_supported_dimension_metric(dimension: &str, metric: &str, filter: &Filter) -> bool {
    metric != "total"
        || if filter.inbox_ids.is_empty() {
            matches!(dimension, "channel" | "source")
        } else {
            dimension == "channel"
        }
}

/// One `contactsByDimension` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DimensionCount {
    pub dimension: String,
    pub count: i64,
    pub unique_contacts: i64,
}

/// `getContactsByDimension`. `dimension` is one of [`DIMENSIONS`] (a column
/// name of the event table).
pub fn contacts_by_dimension(
    conn: &Connection,
    range: &TimeRange,
    dimension: &str,
    metric: &str,
) -> repo::Result<Vec<DimensionCount>> {
    let Some((from, to)) = range.bounds() else {
        return Ok(Vec::new());
    };
    let column = match dimension {
        "country" => r#""country""#,
        "channel" => r#""channel""#,
        "source" => r#""source""#,
        _ => r#""language""#,
    };
    let mut binds = vec![Sql::Integer(range.workspace_id)];
    let sql = if metric == "total" && !range.filter.inbox_ids.is_empty() {
        // `inboxFilteredChannelTotals`: per channel, the current per-inbox
        // count minus the balance after `to`.
        let current = range
            .filter
            .sql(r#"i."channel""#, r#"ics."inboxId""#, &mut binds);
        binds.push(Sql::Integer(range.workspace_id));
        let since = range.filter.event_sql(&mut binds);
        binds.push(Sql::Integer(to));
        format!(
            r#"SELECT current.channel, current.total - COALESCE(since.net, 0), current.total - COALESCE(since.net, 0)
               FROM (
                 SELECT i."channel" AS channel, SUM(ics."totalContacts") AS total
                 FROM "InboxContactStat" ics INNER JOIN "Inbox" i ON i."id" = ics."inboxId"
                 WHERE i."workspaceId" = ?{current}
                 GROUP BY 1
               ) current
               LEFT JOIN (
                 SELECT "channel" AS channel, {BALANCE} AS net FROM "AnalyticsContactEvent"
                 WHERE "workspaceId" = ?{since} AND "occurredAt" > ?
                   AND "eventType" IN ('contact_created', 'contact_deleted')
                 GROUP BY 1
               ) since ON since.channel = current.channel
               WHERE current.total - COALESCE(since.net, 0) > 0
               ORDER BY 2 DESC, 1"#
        )
    } else if metric == "total" {
        let narrow = range.filter.event_sql(&mut binds);
        binds.push(Sql::Integer(to));
        format!(
            r#"SELECT COALESCE(NULLIF({column}, ''), ''), {BALANCE}, {BALANCE}
               FROM "AnalyticsContactEvent"
               WHERE "workspaceId" = ?{narrow} AND "occurredAt" <= ?
                 AND "eventType" IN ('contact_created', 'contact_deleted')
               GROUP BY 1
               HAVING {BALANCE} > 0
               ORDER BY 2 DESC, 1"#
        )
    } else {
        let narrow = range.filter.event_sql(&mut binds);
        binds.extend([Sql::Integer(from), Sql::Integer(to)]);
        format!(
            r#"SELECT {column}, COUNT(*), COUNT(DISTINCT "contactId")
               FROM "AnalyticsContactEvent"
               WHERE "workspaceId" = ?{narrow} AND "occurredAt" >= ? AND "occurredAt" <= ?
                 AND "eventType" = 'contact_created'
                 AND {column} IS NOT NULL AND {column} != ''
               GROUP BY 1
               ORDER BY 2 DESC, 1"#
        )
    };
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params_from_iter(binds.iter()), |row| {
            Ok(DimensionCount {
                dimension: row.get(0)?,
                count: row.get(1)?,
                unique_contacts: row.get(2)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// `macRepository.countActiveContactsByWorkspace`: distinct contacts with an
/// activity hour in `[from, to]`. The channel filter resolves inbox ids by a
/// subquery, so unfiltered reads keep the rows of deleted inboxes.
pub fn active_contacts_count(conn: &Connection, range: &TimeRange) -> repo::Result<i64> {
    let Some((from, to)) = range.bounds() else {
        return Ok(0);
    };
    let mut binds = vec![
        Sql::Integer(range.workspace_id),
        Sql::Integer(from),
        Sql::Integer(to),
    ];
    let mut sql = String::from(
        r#"SELECT COUNT(DISTINCT "contactId") FROM "ContactActiveHourly"
           WHERE "workspaceId" = ? AND "hourBucket" >= ? AND "hourBucket" <= ?"#,
    );
    let filter = &range.filter;
    if !filter.inbox_ids.is_empty() {
        sql.push_str(&format!(
            r#" AND "inboxId" IN ({})"#,
            super::marks(filter.inbox_ids.len())
        ));
        binds.extend(filter.inbox_ids.iter().copied().map(Sql::Integer));
    }
    if !filter.channels.is_empty() {
        sql.push_str(&format!(
            r#" AND "inboxId" IN (SELECT "id" FROM "Inbox" WHERE "workspaceId" = ? AND "channel" IN ({}))"#,
            super::marks(filter.channels.len())
        ));
        binds.push(Sql::Integer(range.workspace_id));
        binds.extend(filter.channels.iter().cloned().map(Sql::Text));
    }
    count(conn, &sql, &binds)
}

/// `macRepository.getActiveContactCountByWorkspaceId`: the `macCount` of the
/// latest `WorkspaceMac` row whose period holds `now` (0 without one).
pub fn current_mac_count(conn: &Connection, workspace_id: i64, now_ms: i64) -> repo::Result<i64> {
    use chatbotx_db::rusqlite::OptionalExtension as _;
    Ok(conn
        .query_row(
            r#"SELECT "macCount" FROM "WorkspaceMac"
               WHERE "workspaceId" = ?1 AND "periodStart" <= ?2 AND "periodEnd" > ?2
               ORDER BY "id" DESC LIMIT 1"#,
            [workspace_id, now_ms],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .unwrap_or(0))
}
