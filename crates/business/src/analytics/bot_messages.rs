//! `botMessageAnalyticsService` / `botMessageStatsRepository` (reads).

use std::collections::HashMap;

use chatbotx_db::repo;
use chatbotx_db::rusqlite::types::Value as Sql;
use chatbotx_db::rusqlite::{Connection, params_from_iter};

use super::{Granularity, TimeRange};

/// `granularityDayHourMinuteSchema`.
pub const GRANULARITIES: &[&str] = &["minute", "hour", "day"];

/// One `BotMessageStats` row. `result: None` leaves the key out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BotRow {
    /// ISO bucket start.
    pub timestamp: String,
    pub has_response: bool,
    pub response_type: Option<String>,
    pub result: Option<String>,
    pub ai_provider: String,
    pub count: i64,
}

/// The SQL ISO start of the bucket of `"occurredAt"`: minute / hour in UTC
/// (`time_bucket` on the raw instant), day / month on the local wall clock.
fn bucket_sql(granularity: &str, range: &TimeRange, month: bool) -> String {
    let local = format!(r#"tz_wall_ms("occurredAt", '{}') / 1000"#, range.tz.name());
    match (granularity, month) {
        ("minute", _) => {
            r#"strftime('%Y-%m-%dT%H:%M:00.000Z', "occurredAt" / 1000, 'unixepoch')"#.into()
        }
        ("hour", _) => {
            r#"strftime('%Y-%m-%dT%H:00:00.000Z', "occurredAt" / 1000, 'unixepoch')"#.into()
        }
        (_, true) => format!("strftime('%Y-%m-01T00:00:00.000Z', {local}, 'unixepoch')"),
        _ => format!("strftime('%Y-%m-%dT00:00:00.000Z', {local}, 'unixepoch')"),
    }
}

/// Rows of `SELECT bucket, responseType, result, aiProvider, COUNT(*)` under
/// `condition`, ordered by bucket.
fn grouped(
    conn: &Connection,
    range: &TimeRange,
    bucket: &str,
    condition: &str,
    has_response: bool,
) -> repo::Result<Vec<BotRow>> {
    let Some((from, to)) = range.bounds() else {
        return Ok(Vec::new());
    };
    let mut binds = vec![Sql::Integer(range.workspace_id)];
    let narrow = range.filter.event_sql(&mut binds);
    binds.extend([Sql::Integer(from), Sql::Integer(to)]);
    let sql = format!(
        r#"SELECT {bucket}, "responseType", "result", "aiProvider", COUNT(*) FROM "AnalyticsBotMessageEvent"
           WHERE "workspaceId" = ?{narrow} AND "occurredAt" >= ? AND "occurredAt" <= ? AND {condition}
           GROUP BY 1, 2, 3, 4
           ORDER BY 1 ASC, 2 ASC, 3 ASC, 4 ASC"#
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params_from_iter(binds.iter()), |row| {
            Ok(BotRow {
                timestamp: row.get(0)?,
                has_response,
                response_type: Some(
                    row.get::<_, Option<String>>(1)?
                        .unwrap_or_else(|| "none".into()),
                ),
                result: row.get(2)?,
                ai_provider: row
                    .get::<_, Option<String>>(3)?
                    .unwrap_or_else(|| "none".into()),
                count: row.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// `getMessagesByResult`. Day (and month past 60 days): one row per bucket
/// and result (`success`, `fallback`), zero-filled; a bucket's rows of one
/// result are summed and keep the first row's `responseType` / `aiProvider`.
/// Minute / hour: the raw groups, no fill.
pub fn by_result(
    conn: &Connection,
    range: &TimeRange,
    granularity: &str,
) -> repo::Result<Vec<BotRow>> {
    let month = granularity == "day" && range.granularity() == Granularity::Month;
    let rows = grouped(
        conn,
        range,
        &bucket_sql(granularity, range, month),
        r#""result" IS NOT NULL"#,
        true,
    )?;
    if granularity != "day" {
        return Ok(rows);
    }
    let mut by_key: HashMap<(String, String), BotRow> = HashMap::new();
    for row in rows {
        let Some(result) = row.result.clone() else {
            continue;
        };
        by_key
            .entry((row.timestamp.clone(), result))
            .and_modify(|existing| existing.count += row.count)
            .or_insert(row);
    }
    let series = if month {
        Granularity::Month
    } else {
        Granularity::Day
    }
    .series(range);
    let mut out = Vec::new();
    for bucket in series {
        for result in ["success", "fallback"] {
            out.push(
                by_key
                    .remove(&(bucket.date.clone(), result.to_string()))
                    .unwrap_or_else(|| BotRow {
                        timestamp: bucket.date.clone(),
                        has_response: true,
                        response_type: Some("none".into()),
                        result: Some(result.into()),
                        ai_provider: "none".into(),
                        count: 0,
                    }),
            );
        }
    }
    Ok(out)
}

/// `getMessagesWithResponse`: answered groups per bucket (day: in local-day
/// order, no fill).
pub fn with_response(
    conn: &Connection,
    range: &TimeRange,
    granularity: &str,
) -> repo::Result<Vec<BotRow>> {
    let rows = grouped(
        conn,
        range,
        &bucket_sql(granularity, range, false),
        r#""hasResponse" = 1"#,
        true,
    )?;
    if granularity != "day" {
        return Ok(rows);
    }
    let days: Vec<String> = Granularity::Day
        .series(range)
        .into_iter()
        .map(|b| b.date)
        .collect();
    Ok(rows
        .into_iter()
        .filter(|row| days.contains(&row.timestamp))
        .collect())
}

/// `getMessagesWithNoResponse`: unanswered messages per bucket and
/// `responseType` (`aiProvider` `none`, no `result`).
pub fn no_response(
    conn: &Connection,
    range: &TimeRange,
    granularity: &str,
) -> repo::Result<Vec<BotRow>> {
    let Some((from, to)) = range.bounds() else {
        return Ok(Vec::new());
    };
    let bucket = bucket_sql(granularity, range, false);
    let mut binds = vec![Sql::Integer(range.workspace_id)];
    let narrow = range.filter.event_sql(&mut binds);
    binds.extend([Sql::Integer(from), Sql::Integer(to)]);
    let sql = format!(
        r#"SELECT {bucket}, "responseType", COUNT(*) FROM "AnalyticsBotMessageEvent"
           WHERE "workspaceId" = ?{narrow} AND "occurredAt" >= ? AND "occurredAt" <= ? AND "hasResponse" = 0
           GROUP BY 1, 2
           ORDER BY 1 ASC, 2 ASC"#
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params_from_iter(binds.iter()), |row| {
            Ok(BotRow {
                timestamp: row.get(0)?,
                has_response: false,
                response_type: row.get(1)?,
                result: None,
                ai_provider: "none".into(),
                count: row.get(2)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// `getAIProviderStats`: AI-agent answers per provider with their share.
pub fn ai_providers(conn: &Connection, range: &TimeRange) -> repo::Result<Vec<(String, i64, f64)>> {
    let Some((from, to)) = range.bounds() else {
        return Ok(Vec::new());
    };
    let mut binds = vec![Sql::Integer(range.workspace_id)];
    let narrow = range.filter.event_sql(&mut binds);
    binds.extend([Sql::Integer(from), Sql::Integer(to)]);
    let sql = format!(
        r#"SELECT "aiProvider", COUNT(*) FROM "AnalyticsBotMessageEvent"
           WHERE "workspaceId" = ?{narrow} AND "occurredAt" >= ? AND "occurredAt" <= ?
             AND "hasResponse" = 1 AND "responseType" = 'ai_agent'
             AND "aiProvider" IS NOT NULL AND "aiProvider" != 'none'
           GROUP BY "aiProvider"
           ORDER BY 2 DESC, 1"#
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params_from_iter(binds.iter()), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let total: i64 = rows.iter().map(|r| r.1).sum();
    Ok(rows
        .into_iter()
        .map(|(provider, count)| {
            let share = if total > 0 {
                count as f64 / total as f64 * 100.0
            } else {
                0.0
            };
            (provider, count, share)
        })
        .collect())
}
