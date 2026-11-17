//! `conversationAnalyticsService` / `conversationStatsRepository` (reads).
//! Per-day rows have no zero fill: only days with events are returned.

use std::collections::HashMap;

use chatbotx_db::repo;
use chatbotx_db::rusqlite::types::Value as Sql;
use chatbotx_db::rusqlite::{Connection, params_from_iter};

use super::{Granularity, TimeRange};

/// One day row (`direction` only for handoffs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DayCount {
    /// ISO date of the local day (`YYYY-MM-DDT00:00:00.000Z`).
    pub timestamp: String,
    pub direction: Option<String>,
    pub count: i64,
}

/// `getFollowUpsByDay` / `getArchivedByDay` / `getAssignedByDay`: events of
/// one type per local day.
pub fn events_by_day(
    conn: &Connection,
    range: &TimeRange,
    event_type: &str,
) -> repo::Result<Vec<DayCount>> {
    by_day(conn, range, "NULL", &[event_type])
}

/// `getHandoffsByDay`: transfers per local day and direction (`to_human` /
/// `to_bot`), ordered by day then direction.
pub fn handoffs_by_day(conn: &Connection, range: &TimeRange) -> repo::Result<Vec<DayCount>> {
    by_day(
        conn,
        range,
        r#"CASE "eventType" WHEN 'conversation_transferred_to_human' THEN 'to_human' ELSE 'to_bot' END"#,
        &[
            "conversation_transferred_to_human",
            "conversation_transferred_to_bot",
        ],
    )
}

fn by_day(
    conn: &Connection,
    range: &TimeRange,
    direction: &str,
    types: &[&str],
) -> repo::Result<Vec<DayCount>> {
    let Some((from, to)) = range.bounds() else {
        return Ok(Vec::new());
    };
    let key = Granularity::Day.key_sql(r#""occurredAt""#, range.tz);
    let mut binds = vec![Sql::Integer(range.workspace_id)];
    let narrow = range.filter.event_sql(&mut binds);
    binds.extend([Sql::Integer(from), Sql::Integer(to)]);
    binds.extend(types.iter().map(|t| Sql::Text((*t).into())));
    let sql = format!(
        r#"SELECT {key}, {direction}, COUNT(*) FROM "AnalyticsConversationEvent"
           WHERE "workspaceId" = ?{narrow} AND "occurredAt" >= ? AND "occurredAt" <= ?
             AND "eventType" IN ({})
           GROUP BY 1, 2
           ORDER BY 1 ASC, 2 ASC"#,
        super::marks(types.len())
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params_from_iter(binds.iter()), |row| {
            Ok(DayCount {
                timestamp: format!("{}T00:00:00.000Z", row.get::<_, String>(0)?),
                direction: row.get(1)?,
                count: row.get(2)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// A current workspace member with a count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberCount {
    pub user_id: i64,
    pub count: i64,
    pub user_name: Option<String>,
    pub user_email: Option<String>,
}

/// `(userId, name, email)` of a workspace member.
pub type Member = (i64, Option<String>, Option<String>);

/// The workspace members (member id order).
pub fn workspace_members(conn: &Connection, workspace_id: i64) -> repo::Result<Vec<Member>> {
    let mut stmt = conn.prepare_cached(
        r#"SELECT wm."userId", u."name", u."email" FROM "WorkspaceMember" wm
           LEFT JOIN "User" u ON u."id" = wm."userId"
           WHERE wm."workspaceId" = ?1 ORDER BY wm."id""#,
    )?;
    let rows = stmt
        .query_map([workspace_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// `getAssignedByAdmin` (`distinct = false`: assignment events) /
/// `getUniqueConversationsByAdmin` (`true`: distinct conversations) per
/// current member (0 without assignments; former members are dropped).
pub fn assigned_by_admin(
    conn: &Connection,
    range: &TimeRange,
    distinct: bool,
) -> repo::Result<Vec<MemberCount>> {
    let mut counts: HashMap<i64, i64> = HashMap::new();
    if let Some((from, to)) = range.bounds() {
        let measure = if distinct {
            r#"COUNT(DISTINCT "conversationId")"#
        } else {
            "COUNT(*)"
        };
        let mut binds = vec![Sql::Integer(range.workspace_id)];
        let narrow = range.filter.event_sql(&mut binds);
        binds.extend([Sql::Integer(from), Sql::Integer(to)]);
        let sql = format!(
            r#"SELECT "toAssignee", {measure} FROM "AnalyticsConversationEvent"
               WHERE "workspaceId" = ?{narrow} AND "occurredAt" >= ? AND "occurredAt" <= ?
                 AND "eventType" = 'conversation_assigned' AND "toAssignee" IS NOT NULL
               GROUP BY "toAssignee""#
        );
        let mut stmt = conn.prepare(&sql)?;
        counts = stmt
            .query_map(params_from_iter(binds.iter()), |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?
            .collect::<Result<HashMap<_, _>, _>>()?;
    }
    Ok(workspace_members(conn, range.workspace_id)?
        .into_iter()
        .map(|(user_id, user_name, user_email)| MemberCount {
            user_id,
            count: counts.get(&user_id).copied().unwrap_or(0),
            user_name,
            user_email,
        })
        .collect())
}
