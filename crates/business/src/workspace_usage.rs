//! `workspace-usage/service.ts` (Q.D11): the per-workspace usage counters
//! (`WorkspaceUsage`). The original keeps a Redis live counter next to the
//! row; here the row is the only counter.

use chatbotx_db::repo::{self, new_id, now_ms};
use chatbotx_db::rusqlite::{Connection, params};

/// `WorkspaceUsageMetric`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceUsageMetric {
    Contacts,
    Channels,
    TeamMembers,
    BotMessages,
    Mac,
}

impl WorkspaceUsageMetric {
    fn column(self) -> &'static str {
        match self {
            WorkspaceUsageMetric::Contacts => "contactsUsed",
            WorkspaceUsageMetric::Channels => "channelsUsed",
            WorkspaceUsageMetric::TeamMembers => "teamMembersUsed",
            WorkspaceUsageMetric::BotMessages => "botMessagesUsed",
            WorkspaceUsageMetric::Mac => "macUsed",
        }
    }
}

/// `increment` (`delta > 0`) / `decrement` (`delta < 0`, never below 0): upserts
/// the workspace's row.
pub fn adjust(
    conn: &Connection,
    workspace_id: i64,
    metric: WorkspaceUsageMetric,
    delta: i64,
) -> repo::Result<()> {
    let column = metric.column();
    let now = now_ms();
    conn.execute(
        &format!(
            r#"INSERT INTO "WorkspaceUsage" ("id", "workspaceId", "{column}", "createdAt", "updatedAt")
               VALUES (?1, ?2, MAX(0, ?3), ?4, ?4)
               ON CONFLICT ("workspaceId") DO UPDATE SET
                 "{column}" = MAX(0, "WorkspaceUsage"."{column}" + ?3), "updatedAt" = ?4"#
        ),
        params![new_id(), workspace_id, delta, now],
    )?;
    Ok(())
}
