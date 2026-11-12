//! Port of `repositories/workspace-purge/` (`index.ts`, `tables.ts`; P3.3).
//!
//! Postgres `ctid` / `FOR UPDATE SKIP LOCKED` become SQLite `rowid` batches (every app
//! table has a rowid). TimescaleDB decompression limits do not exist. Table names are
//! checked against the purge lists, so no caller text reaches the SQL.
#![allow(
    unused_variables,
    unused_imports,
    dead_code,
    unreachable_code,
    clippy::all
)]

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
    todo!("P3.3")
}

/// `hasWorkspaceRows`.
pub fn has_workspace_rows(conn: &Connection, table: &str, workspace_id: i64) -> Result<bool> {
    todo!("P3.3")
}

/// `deletePartitionedBatch`.
pub fn delete_partitioned_batch(
    conn: &Connection,
    spec: &PartitionedWorkspaceTable,
    workspace_id: i64,
    limit: i64,
) -> Result<usize> {
    todo!("P3.3")
}

/// `hasPartitionedWorkspaceRows`.
pub fn has_partitioned_workspace_rows(
    conn: &Connection,
    spec: &PartitionedWorkspaceTable,
    workspace_id: i64,
) -> Result<bool> {
    todo!("P3.3")
}

/// `findNextAnalyticsRowAt`: the oldest `occurredAt` (unix ms) of the workspace's rows.
pub fn find_next_analytics_row_at(
    conn: &Connection,
    table: &str,
    workspace_id: i64,
) -> Result<Option<i64>> {
    todo!("P3.3")
}

/// `deleteAnalyticsSlice`: deletes the workspace's rows with `from <= occurredAt < to`.
pub fn delete_analytics_slice(
    conn: &Connection,
    table: &str,
    workspace_id: i64,
    from: i64,
    to: i64,
) -> Result<usize> {
    todo!("P3.3")
}

/// `listMessageConversationPage`: up to `limit` distinct conversation ids, ascending.
pub fn list_message_conversation_page(
    conn: &Connection,
    table: &str,
    workspace_id: i64,
    limit: i64,
) -> Result<Vec<i64>> {
    todo!("P3.3")
}

/// `deleteMessagesAndAttachments`: deletes the messages and attachments of the
/// conversations; returns the total count.
pub fn delete_messages_and_attachments(
    conn: &Connection,
    workspace_id: i64,
    conversation_ids: &[i64],
) -> Result<usize> {
    todo!("P3.3")
}
