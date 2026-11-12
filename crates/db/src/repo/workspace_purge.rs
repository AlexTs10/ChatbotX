//! Port of `repositories/workspace-purge/` (`index.ts`, `tables.ts`; P3.3).
//!
//! Postgres `ctid` / `FOR UPDATE SKIP LOCKED` become SQLite `rowid` batches (every app
//! table has a rowid). TimescaleDB decompression limits do not exist. Table names are
//! checked against the purge lists, so no caller text reaches the SQL.

use rusqlite::{Connection, OptionalExtension};

use super::{Error, Result, id_list};

/// `HEAVY_WORKSPACE_TABLES` (drain order).
pub const HEAVY_WORKSPACE_TABLES: &[&str] = &[
    "ErrorLog",
    "AuditLog",
    "InboundWebhookRequest",
    "CommentAutomationEvent",
    "CommentAutomationReply",
    "CommentModerationLog",
    "TriggerStat",
    "FlowNodeStat",
    "MagicLinkStat",
    "RefLinkStat",
    "MetaCapiEvent",
    "AdsConversionEvent",
    "GoogleAdsConversionEvent",
    "ContactReferralEvent",
    "LeaderboardAuditLog",
    "LeaderboardParticipant",
    "AIConversationEmbedding",
    "AIEmbedding",
    "TriggerExecution",
    "FlowRun",
    "FlowVersion",
    "ContactOnSmartDelay",
    "Conversation",
    "Contact",
];

/// How a partitioned table finds the workspace's rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartitionSelector {
    /// Through `Broadcast.workspaceId` (`ContactOnBroadcast`).
    BroadcastWorkspace,
    /// Its own `workspaceId` column.
    Workspace,
}

/// One `PARTITIONED_WORKSPACE_TABLES` entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PartitionedWorkspaceTable {
    pub primary_key: &'static [&'static str],
    pub selector: PartitionSelector,
    pub table: &'static str,
}

/// `PARTITIONED_WORKSPACE_TABLES`.
pub const PARTITIONED_WORKSPACE_TABLES: &[PartitionedWorkspaceTable] = &[
    PartitionedWorkspaceTable {
        primary_key: &["broadcastId", "contactId"],
        selector: PartitionSelector::BroadcastWorkspace,
        table: "ContactOnBroadcast",
    },
    PartitionedWorkspaceTable {
        primary_key: &["id", "workspaceId"],
        selector: PartitionSelector::Workspace,
        table: "SequenceDispatch",
    },
    PartitionedWorkspaceTable {
        primary_key: &["id", "workspaceId"],
        selector: PartitionSelector::Workspace,
        table: "PipelineItemActivity",
    },
    PartitionedWorkspaceTable {
        primary_key: &["workspaceId", "hourBucket", "contactInboxId"],
        selector: PartitionSelector::Workspace,
        table: "ContactActiveHourly",
    },
    PartitionedWorkspaceTable {
        primary_key: &["workspaceId", "periodStart", "contactInboxId"],
        selector: PartitionSelector::Workspace,
        table: "ContactActiveMonthly",
    },
];

/// `WORKSPACE_ANALYTICS_HYPERTABLES`.
pub const WORKSPACE_ANALYTICS_HYPERTABLES: &[&str] = &[
    "AnalyticsAgentResponseEvent",
    "AnalyticsBotMessageEvent",
    "AnalyticsContactEvent",
    "AnalyticsConversationEvent",
    "AnalyticsMessageEvent",
];

/// `NO_FK_WORKSPACE_TABLES`: `(stage, table)`.
pub const NO_FK_WORKSPACE_TABLES: &[(&str, &str)] = &[
    ("ctid", "CoexistSyncRun"),
    ("contactActivity", "ContactActivity"),
    ("contactActivity", "ContactKeywordMessage"),
    ("contactActivity", "ContactState"),
    ("ctid", "FlowAnalyticsSession"),
    ("partitioned", "ContactActiveHourly"),
    ("partitioned", "ContactActiveMonthly"),
    ("ctid", "MessageCleanup"),
    ("contactInboxPost", "ContactInboxPost"),
    ("leaderboard", "LeaderboardScoreEvent"),
    ("hypertable", "Message"),
    ("hypertable", "Attachment"),
    ("retainedBilling", "WorkspaceMac"),
    ("retainedBilling", "WorkspaceBotMessageUsage"),
    ("retainedBilling", "WorkspaceDeletionRecord"),
    ("retainedBilling", "ResourceDeletionLog"),
];

fn known(table: &str, allowed: &[&str]) -> Result<()> {
    if allowed.contains(&table) {
        Ok(())
    } else {
        Err(Error::msg(format!("not a workspace purge table: {table}")))
    }
}

fn purge_table(table: &str) -> Result<()> {
    if HEAVY_WORKSPACE_TABLES.contains(&table)
        || WORKSPACE_ANALYTICS_HYPERTABLES.contains(&table)
        || PARTITIONED_WORKSPACE_TABLES
            .iter()
            .any(|t| t.table == table)
        || NO_FK_WORKSPACE_TABLES.iter().any(|(_, t)| *t == table)
    {
        Ok(())
    } else {
        Err(Error::msg(format!("not a workspace purge table: {table}")))
    }
}

/// `deleteWorkspaceRowsBatch`: deletes up to `limit` rows of the workspace (heavy tables,
/// `CoexistSyncRun`, `FlowAnalyticsSession`, `MessageCleanup`); returns the count.
pub fn delete_workspace_rows_batch(
    conn: &Connection,
    table: &str,
    workspace_id: i64,
    limit: i64,
) -> Result<usize> {
    if !HEAVY_WORKSPACE_TABLES.contains(&table) {
        known(
            table,
            &["CoexistSyncRun", "FlowAnalyticsSession", "MessageCleanup"],
        )?;
    }
    let sql = format!(
        "DELETE FROM \"{table}\" WHERE rowid IN (SELECT rowid FROM \"{table}\" WHERE \"workspaceId\" = ?1 LIMIT ?2)"
    );
    Ok(conn.execute(&sql, rusqlite::params![workspace_id, limit])?)
}

/// `hasWorkspaceRows`.
pub fn has_workspace_rows(conn: &Connection, table: &str, workspace_id: i64) -> Result<bool> {
    purge_table(table)?;
    let sql = format!("SELECT 1 FROM \"{table}\" WHERE \"workspaceId\" = ?1 LIMIT 1");
    Ok(conn.prepare_cached(&sql)?.exists([workspace_id])?)
}

/// `deletePartitionedBatch`.
pub fn delete_partitioned_batch(
    conn: &Connection,
    spec: &PartitionedWorkspaceTable,
    workspace_id: i64,
    limit: i64,
) -> Result<usize> {
    known(
        spec.table,
        &PARTITIONED_WORKSPACE_TABLES
            .iter()
            .map(|t| t.table)
            .collect::<Vec<_>>(),
    )?;
    let sql = match spec.selector {
        PartitionSelector::BroadcastWorkspace => format!(
            "DELETE FROM \"{t}\" WHERE rowid IN (SELECT recipient.rowid FROM \"ContactOnBroadcast\" AS recipient \
             INNER JOIN \"Broadcast\" AS broadcast ON broadcast.\"id\" = recipient.\"broadcastId\" \
             WHERE broadcast.\"workspaceId\" = ?1 LIMIT ?2)",
            t = spec.table
        ),
        PartitionSelector::Workspace => format!(
            "DELETE FROM \"{t}\" WHERE rowid IN (SELECT rowid FROM \"{t}\" WHERE \"workspaceId\" = ?1 LIMIT ?2)",
            t = spec.table
        ),
    };
    Ok(conn.execute(&sql, rusqlite::params![workspace_id, limit])?)
}

/// `hasPartitionedWorkspaceRows`.
pub fn has_partitioned_workspace_rows(
    conn: &Connection,
    spec: &PartitionedWorkspaceTable,
    workspace_id: i64,
) -> Result<bool> {
    known(
        spec.table,
        &PARTITIONED_WORKSPACE_TABLES
            .iter()
            .map(|t| t.table)
            .collect::<Vec<_>>(),
    )?;
    let sql = match spec.selector {
        PartitionSelector::BroadcastWorkspace => {
            "SELECT 1 FROM \"ContactOnBroadcast\" AS recipient \
             INNER JOIN \"Broadcast\" AS broadcast ON broadcast.\"id\" = recipient.\"broadcastId\" \
             WHERE broadcast.\"workspaceId\" = ?1 LIMIT 1"
                .to_owned()
        }
        PartitionSelector::Workspace => format!(
            "SELECT 1 FROM \"{}\" WHERE \"workspaceId\" = ?1 LIMIT 1",
            spec.table
        ),
    };
    Ok(conn.prepare_cached(&sql)?.exists([workspace_id])?)
}

/// `findNextAnalyticsRowAt`: the oldest `occurredAt` (unix ms) of the workspace's rows.
pub fn find_next_analytics_row_at(
    conn: &Connection,
    table: &str,
    workspace_id: i64,
) -> Result<Option<i64>> {
    known(table, WORKSPACE_ANALYTICS_HYPERTABLES)?;
    let sql = format!(
        "SELECT \"occurredAt\" FROM \"{table}\" WHERE \"workspaceId\" = ?1 ORDER BY \"occurredAt\" ASC LIMIT 1"
    );
    Ok(conn
        .prepare_cached(&sql)?
        .query_row([workspace_id], |r| r.get(0))
        .optional()?)
}

/// `deleteAnalyticsSlice`: deletes the workspace's rows with `from <= occurredAt < to`.
pub fn delete_analytics_slice(
    conn: &Connection,
    table: &str,
    workspace_id: i64,
    from: i64,
    to: i64,
) -> Result<usize> {
    known(table, WORKSPACE_ANALYTICS_HYPERTABLES)?;
    let sql = format!(
        "DELETE FROM \"{table}\" WHERE \"workspaceId\" = ?1 AND \"occurredAt\" >= ?2 AND \"occurredAt\" < ?3"
    );
    Ok(conn.execute(&sql, rusqlite::params![workspace_id, from, to])?)
}

/// `listMessageConversationPage`: up to `limit` distinct conversation ids, ascending.
pub fn list_message_conversation_page(
    conn: &Connection,
    table: &str,
    workspace_id: i64,
    limit: i64,
) -> Result<Vec<i64>> {
    known(table, &["Message", "Attachment"])?;
    let sql = format!(
        "SELECT DISTINCT \"conversationId\" FROM \"{table}\" WHERE \"workspaceId\" = ?1 \
         ORDER BY \"conversationId\" ASC LIMIT ?2"
    );
    let mut stmt = conn.prepare_cached(&sql)?;
    let ids = stmt
        .query_map(rusqlite::params![workspace_id, limit], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<i64>>>()?;
    Ok(ids)
}

/// `deleteMessagesAndAttachments`: deletes the messages and attachments of the
/// conversations; returns the total count.
pub fn delete_messages_and_attachments(
    conn: &Connection,
    workspace_id: i64,
    conversation_ids: &[i64],
) -> Result<usize> {
    let list = id_list(conversation_ids);
    let messages = conn.execute(
        "DELETE FROM \"Message\" WHERE \"workspaceId\" = ?1 AND \"conversationId\" IN (SELECT value FROM json_each(?2))",
        rusqlite::params![workspace_id, list],
    )?;
    let attachments = conn.execute(
        "DELETE FROM \"Attachment\" WHERE \"workspaceId\" = ?1 AND \"conversationId\" IN (SELECT value FROM json_each(?2))",
        rusqlite::params![workspace_id, list],
    )?;
    Ok(messages + attachments)
}
