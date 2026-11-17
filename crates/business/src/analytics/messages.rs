//! `messageAnalyticsService` / `messageStatsRepository` and
//! `agentResponseAnalyticsService` / `agentResponseStatsRepository` (reads).

use std::collections::HashMap;

use chatbotx_db::repo;
use chatbotx_db::rusqlite::types::Value as Sql;
use chatbotx_db::rusqlite::{Connection, params_from_iter};

use super::conversations::{assigned_by_admin, workspace_members};
use super::{Granularity, TimeRange};

/// One `messagesBySender` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SenderCount {
    pub timestamp: String,
    pub channel: String,
    pub sender_type: String,
    pub count: i64,
}

/// `getMessagesBySender`: messages per local day (or month: `granularity
/// month`, or always past 60 days), channel and sender type; events without
/// a channel or sender type are not counted.
pub fn messages_by_sender(
    conn: &Connection,
    range: &TimeRange,
    month: bool,
) -> repo::Result<Vec<SenderCount>> {
    let Some((from, to)) = range.bounds() else {
        return Ok(Vec::new());
    };
    let granularity = if month || range.granularity() == Granularity::Month {
        Granularity::Month
    } else {
        Granularity::Day
    };
    let key = granularity.key_sql(r#""occurredAt""#, range.tz);
    let mut binds = vec![Sql::Integer(range.workspace_id)];
    let narrow = range.filter.event_sql(&mut binds);
    binds.extend([Sql::Integer(from), Sql::Integer(to)]);
    let sql = format!(
        r#"SELECT {key}, "channel", "senderType", COUNT(*) FROM "AnalyticsMessageEvent"
           WHERE "workspaceId" = ?{narrow} AND "occurredAt" >= ? AND "occurredAt" <= ?
             AND "senderType" IS NOT NULL AND "channel" IS NOT NULL
           GROUP BY 1, 2, 3
           ORDER BY 1 ASC, 2 ASC, 3 ASC"#
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params_from_iter(binds.iter()), |row| {
            let key: String = row.get(0)?;
            Ok(SenderCount {
                timestamp: match granularity {
                    Granularity::Day => format!("{key}T00:00:00.000Z"),
                    Granularity::Month => format!("{key}-01T00:00:00.000Z"),
                },
                channel: row.get(1)?,
                sender_type: row.get(2)?,
                count: row.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// `adminId -> count` of `getMessagesByAdminCounts` (`unique = false`:
/// `message_human_sent` events) / `getUniqueContactsByAdminCounts` (`true`:
/// distinct contacts of human-sent messages).
fn counts_by_admin(
    conn: &Connection,
    range: &TimeRange,
    unique: bool,
) -> repo::Result<HashMap<i64, i64>> {
    let Some((from, to)) = range.bounds() else {
        return Ok(HashMap::new());
    };
    let (measure, kind) = if unique {
        (
            r#"COUNT(DISTINCT "contactId")"#,
            r#""senderType" = 'human'"#,
        )
    } else {
        ("COUNT(*)", r#""eventType" = 'message_human_sent'"#)
    };
    let mut binds = vec![Sql::Integer(range.workspace_id)];
    let narrow = range.filter.event_sql(&mut binds);
    binds.extend([Sql::Integer(from), Sql::Integer(to)]);
    let sql = format!(
        r#"SELECT "adminId", {measure} FROM "AnalyticsMessageEvent"
           WHERE "workspaceId" = ?{narrow} AND "occurredAt" >= ? AND "occurredAt" <= ?
             AND {kind} AND "adminId" IS NOT NULL
           GROUP BY "adminId""#
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params_from_iter(binds.iter()), |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?
        .collect::<Result<HashMap<_, _>, _>>()?;
    Ok(rows)
}

/// A current member with a count (`messagesByAdmin`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminCount {
    pub admin_id: i64,
    pub count: i64,
    pub user_name: Option<String>,
    pub user_email: Option<String>,
}

/// `getMessagesByAdmin`: human-sent messages per current member.
pub fn messages_by_admin(conn: &Connection, range: &TimeRange) -> repo::Result<Vec<AdminCount>> {
    let counts = counts_by_admin(conn, range, false)?;
    Ok(workspace_members(conn, range.workspace_id)?
        .into_iter()
        .map(|(admin_id, user_name, user_email)| AdminCount {
            admin_id,
            count: counts.get(&admin_id).copied().unwrap_or(0),
            user_name,
            user_email,
        })
        .collect())
}

/// `AgentResponseAverages` (seconds, rounded; `None` without replies).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResponseAverages {
    pub avg_response_seconds: Option<i64>,
    pub avg_working_response_seconds: Option<i64>,
    pub avg_first_response_seconds: Option<i64>,
    pub avg_working_first_response_seconds: Option<i64>,
    pub response_count: i64,
    pub first_response_count: i64,
    pub response_sla_eligible_count: i64,
    pub response_sla_met_count: i64,
    pub first_response_sla_eligible_count: i64,
    pub first_response_sla_met_count: i64,
}

impl ResponseAverages {
    /// `slaPercent(met, eligible)`: one decimal, `None` when nothing was
    /// eligible.
    pub fn sla_percent(met: i64, eligible: i64) -> Option<f64> {
        (eligible != 0).then(|| (met as f64 / eligible as f64 * 1000.0).round() / 10.0)
    }
    pub fn response_sla_met_percent(&self) -> Option<f64> {
        Self::sla_percent(
            self.response_sla_met_count,
            self.response_sla_eligible_count,
        )
    }
    pub fn first_response_sla_met_percent(&self) -> Option<f64> {
        Self::sla_percent(
            self.first_response_sla_met_count,
            self.first_response_sla_eligible_count,
        )
    }
}

/// `measuresSql`: the SLA counts only while the workspace has an enabled
/// `ResponseSlaPolicy` (the workspace id is the first bind).
const MEASURES: &str = r#"
  CAST(ROUND(AVG("responseSeconds")) AS INTEGER),
  CAST(ROUND(AVG("workingResponseSeconds")) AS INTEGER),
  CAST(ROUND(AVG(CASE WHEN "isFirstResponse" THEN "responseSeconds" END)) AS INTEGER),
  CAST(ROUND(AVG(CASE WHEN "isFirstResponse" THEN "workingResponseSeconds" END)) AS INTEGER),
  COUNT(*),
  COALESCE(SUM("isFirstResponse"), 0),
  COALESCE(SUM("slaMet" IS NOT NULL AND sla.on_), 0),
  COALESCE(SUM(COALESCE("slaMet", 0) AND sla.on_), 0),
  COALESCE(SUM("isFirstResponse" AND "slaMet" IS NOT NULL AND sla.on_), 0),
  COALESCE(SUM("isFirstResponse" AND COALESCE("slaMet", 0) AND sla.on_), 0)"#;

fn averages(
    row: &chatbotx_db::rusqlite::Row<'_>,
    at: usize,
) -> chatbotx_db::rusqlite::Result<ResponseAverages> {
    Ok(ResponseAverages {
        avg_response_seconds: row.get(at)?,
        avg_working_response_seconds: row.get(at + 1)?,
        avg_first_response_seconds: row.get(at + 2)?,
        avg_working_first_response_seconds: row.get(at + 3)?,
        response_count: row.get(at + 4)?,
        first_response_count: row.get(at + 5)?,
        response_sla_eligible_count: row.get(at + 6)?,
        response_sla_met_count: row.get(at + 7)?,
        first_response_sla_eligible_count: row.get(at + 8)?,
        first_response_sla_met_count: row.get(at + 9)?,
    })
}

/// `(select, binds)` of the agent response measures grouped by `group`.
fn response_query(range: &TimeRange, from: i64, to: i64, group: &str) -> (String, Vec<Sql>) {
    let mut binds = vec![
        Sql::Integer(range.workspace_id),
        Sql::Integer(range.workspace_id),
    ];
    let narrow = range.filter.event_sql(&mut binds);
    binds.extend([Sql::Integer(from), Sql::Integer(to)]);
    let sql = format!(
        r#"SELECT {group}, {MEASURES}
           FROM "AnalyticsAgentResponseEvent",
             (SELECT EXISTS (SELECT 1 FROM "ResponseSlaPolicy" WHERE "workspaceId" = ? AND "enabled") AS on_) sla
           WHERE "workspaceId" = ?{narrow} AND "occurredAt" >= ? AND "occurredAt" <= ?"#
    );
    (sql, binds)
}

/// `getAveragesByAdmin`.
fn averages_by_admin(
    conn: &Connection,
    range: &TimeRange,
) -> repo::Result<HashMap<i64, ResponseAverages>> {
    let Some((from, to)) = range.bounds() else {
        return Ok(HashMap::new());
    };
    let (sql, binds) = response_query(range, from, to, r#""adminId""#);
    let mut stmt = conn.prepare(&format!(r#"{sql} GROUP BY "adminId""#))?;
    let rows = stmt
        .query_map(params_from_iter(binds.iter()), |row| {
            Ok((row.get(0)?, averages(row, 1)?))
        })?
        .collect::<Result<HashMap<_, _>, _>>()?;
    Ok(rows)
}

/// One `humanAgentStats` row.
#[derive(Debug, Clone, PartialEq)]
pub struct HumanAgentStats {
    pub admin_id: i64,
    pub assigned_conversations: i64,
    pub messages_sent: i64,
    pub unique_contacts: i64,
    pub user_name: Option<String>,
    pub user_email: Option<String>,
    pub response: ResponseAverages,
}

/// `getHumanAgentStats`: per current member, messages sent, distinct
/// contacts messaged, assignments and response averages.
pub fn human_agent_stats(
    conn: &Connection,
    range: &TimeRange,
) -> repo::Result<Vec<HumanAgentStats>> {
    let messages = counts_by_admin(conn, range, false)?;
    let contacts = counts_by_admin(conn, range, true)?;
    let assigned: HashMap<i64, i64> = assigned_by_admin(conn, range, false)?
        .into_iter()
        .map(|row| (row.user_id, row.count))
        .collect();
    let mut responses = averages_by_admin(conn, range)?;
    Ok(workspace_members(conn, range.workspace_id)?
        .into_iter()
        .map(|(admin_id, user_name, user_email)| HumanAgentStats {
            admin_id,
            assigned_conversations: assigned.get(&admin_id).copied().unwrap_or(0),
            messages_sent: messages.get(&admin_id).copied().unwrap_or(0),
            unique_contacts: contacts.get(&admin_id).copied().unwrap_or(0),
            user_name,
            user_email,
            response: responses.remove(&admin_id).unwrap_or_default(),
        })
        .collect())
}

/// One `agentResponseSeries` row.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentResponseDay {
    pub admin_id: i64,
    /// `YYYY-MM-DD` in the request timezone.
    pub date: String,
    pub user_name: Option<String>,
    pub user_email: Option<String>,
    pub response: ResponseAverages,
}

/// `getAgentResponseSeries`: per (agent, local day) with replies, ordered by
/// day then agent; former members are dropped.
pub fn agent_response_series(
    conn: &Connection,
    range: &TimeRange,
) -> repo::Result<Vec<AgentResponseDay>> {
    let Some((from, to)) = range.bounds() else {
        return Ok(Vec::new());
    };
    let key = Granularity::Day.key_sql(r#""occurredAt""#, range.tz);
    let (sql, binds) = response_query(range, from, to, &format!(r#""adminId", {key}"#));
    let mut stmt = conn.prepare(&format!("{sql} GROUP BY 1, 2 ORDER BY 2 ASC, 1 ASC"))?;
    let daily = stmt
        .query_map(params_from_iter(binds.iter()), |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                averages(row, 2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let members: HashMap<i64, (Option<String>, Option<String>)> =
        workspace_members(conn, range.workspace_id)?
            .into_iter()
            .map(|(id, name, email)| (id, (name, email)))
            .collect();
    Ok(daily
        .into_iter()
        .filter_map(|(admin_id, date, response)| {
            let (user_name, user_email) = members.get(&admin_id)?.clone();
            Some(AgentResponseDay {
                admin_id,
                date,
                user_name,
                user_email,
                response,
            })
        })
        .collect())
}
