//! Port of `repositories/conversation/repository.ts` (P3.2b).
//!
//! `closeHumanReplyWindow` runs on the single writer: one write job is the Postgres
//! `FOR NO KEY UPDATE` claim (a second reply runs after the first commits).

use rusqlite::Connection;
use rusqlite::types::Value;

use super::models::Conversation;
use super::sql::{Sql, opt_int};
use super::{Model, Result, new_id, now_ms};

/// `UPDATE "Conversation" SET <set>, "updatedAt" = now WHERE <scope + guard> RETURNING *`.
fn guarded_assign(
    conn: &Connection,
    workspace_id: i64,
    conversation_id: i64,
    set: Vec<(&'static str, Value)>,
) -> Result<Vec<Conversation>> {
    let mut q = Sql::raw("UPDATE \"Conversation\" SET ");
    for (col, value) in set {
        q.push(&format!("\"{col}\" = "));
        q.push_bind(value);
        q.push(", ");
    }
    q.push("\"updatedAt\" = ");
    q.push_bind(now_ms());
    q.push(" WHERE \"workspaceId\" = ");
    q.push_bind(workspace_id);
    q.push(" AND \"id\" = ");
    q.push_bind(conversation_id);
    q.push(&format!(
        " AND \"assignedUserId\" IS NULL AND \"assignedInboxTeamId\" IS NULL RETURNING {}",
        Conversation::columns_sql(None)
    ));
    q.models(conn)
}

/// Input of `assignTargetIfUnassigned` (`assigned_at` `None` = now).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AssignTargetInput {
    pub workspace_id: i64,
    pub conversation_id: i64,
    pub assigned_inbox_team_id: Option<i64>,
    pub assigned_user_id: Option<i64>,
    pub assigned_at: Option<i64>,
}

/// `CloseHumanReplyWindowResult`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CloseHumanReplyWindowResult {
    /// Start of the window this call closed (`None` when none was closed).
    pub closed_awaiting_since: Option<i64>,
    pub previous_admin_replied_at: Option<i64>,
    pub assigned_user_id: Option<i64>,
    pub assigned_user_at: Option<i64>,
    pub closed_response_due_at: Option<i64>,
    pub closed_response_sla_target_seconds: Option<i64>,
}

/// `insertDmConversationsIfAbsent`: opens the DM (`sourceId` NULL) of each contact that
/// has none (a conflict is skipped).
pub fn insert_dm_conversations_if_absent(
    conn: &Connection,
    workspace_id: i64,
    contact_ids: &[i64],
) -> Result<()> {
    let mut stmt = conn.prepare_cached(
        "INSERT INTO \"Conversation\" (\"id\", \"workspaceId\", \"contactId\", \"sourceId\") \
         VALUES (?1, ?2, ?3, NULL) ON CONFLICT DO NOTHING",
    )?;
    for contact_id in contact_ids {
        stmt.execute(rusqlite::params![new_id(), workspace_id, contact_id])?;
    }
    Ok(())
}

/// `assignUserIfUnassigned`: claims the conversation for `user_id` while neither a user
/// nor a team is assigned (`assigned_at` `None` = now). Empty = did not claim.
pub fn assign_user_if_unassigned(
    conn: &Connection,
    workspace_id: i64,
    conversation_id: i64,
    user_id: i64,
    assigned_at: Option<i64>,
) -> Result<Vec<Conversation>> {
    guarded_assign(
        conn,
        workspace_id,
        conversation_id,
        vec![
            ("assignedUserId", Value::Integer(user_id)),
            (
                "assignedUserAt",
                Value::Integer(assigned_at.unwrap_or_else(now_ms)),
            ),
        ],
    )
}

/// `assignTargetIfUnassigned`: `assignedUserAt` moves only when a user is assigned.
pub fn assign_target_if_unassigned(
    conn: &Connection,
    input: &AssignTargetInput,
) -> Result<Vec<Conversation>> {
    let mut set = vec![
        ("assignedInboxTeamId", opt_int(input.assigned_inbox_team_id)),
        ("assignedUserId", opt_int(input.assigned_user_id)),
    ];
    // A team-only assignment never moves the measurement start.
    if input.assigned_user_id.is_some() {
        set.push((
            "assignedUserAt",
            Value::Integer(input.assigned_at.unwrap_or_else(now_ms)),
        ));
    }
    guarded_assign(conn, input.workspace_id, input.conversation_id, set)
}

/// `closeHumanReplyWindow`: records a human reply at `replied_at` and closes the DM
/// waiting window that opened at or before it. `None` when the conversation is not in
/// the workspace.
pub fn close_human_reply_window(
    conn: &Connection,
    workspace_id: i64,
    conversation_id: i64,
    replied_at: i64,
) -> Result<Option<CloseHumanReplyWindowResult>> {
    type Old = (
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
    );
    let old: Option<Old> = Sql::new(
        "SELECT \"awaitingHumanReplySince\", \"adminRepliedAt\", \"assignedUserId\", \"assignedUserAt\", \
         \"responseDueAt\", \"responseSlaTargetSeconds\" FROM \"Conversation\" WHERE \"id\" = ? AND \"workspaceId\" = ?",
        vec![Value::Integer(conversation_id), Value::Integer(workspace_id)],
    )
    .query_map(conn, |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?))
    })?
    .into_iter()
    .next();
    let Some((since, admin_replied_at, assigned_user_id, assigned_user_at, due_at, target)) = old
    else {
        return Ok(None);
    };
    // A window opened after the reply (`since > replied_at`) stays open.
    let closes = since.is_some_and(|s| s <= replied_at);
    let clear = if closes {
        ", \"awaitingHumanReplySince\" = NULL, \"responseDueAt\" = NULL, \
         \"responseSlaBreachingAt\" = NULL, \"responseSlaTargetSeconds\" = NULL"
    } else {
        ""
    };
    let q = Sql::new(
        format!(
            "UPDATE \"Conversation\" SET \"adminRepliedAt\" = ?1, \"agentLastReadAt\" = ?1, \
             \"lastActivityAt\" = ?1, \
             \"awaitingHumanReplyClosedAt\" = max(coalesce(\"awaitingHumanReplyClosedAt\", ?1), ?1){clear} \
             WHERE \"id\" = ?2"
        ),
        vec![Value::Integer(replied_at), Value::Integer(conversation_id)],
    );
    q.execute(conn)?;
    let closed = |v: Option<i64>| if closes { v } else { None };
    Ok(Some(CloseHumanReplyWindowResult {
        closed_awaiting_since: closed(since),
        previous_admin_replied_at: admin_replied_at,
        assigned_user_id,
        assigned_user_at,
        closed_response_due_at: closed(due_at),
        closed_response_sla_target_seconds: closed(target),
    }))
}
