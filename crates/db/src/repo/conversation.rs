//! Port of `repositories/conversation/repository.ts` (P3.2b).
//!
//! `closeHumanReplyWindow` runs on the single writer: one write job is the Postgres
//! `FOR NO KEY UPDATE` claim (a second reply runs after the first commits).

use rusqlite::Connection;

use super::Result;
use super::models::Conversation;

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
    _conn: &Connection,
    _workspace_id: i64,
    _contact_ids: &[i64],
) -> Result<()> {
    todo!("P3.2b")
}

/// `assignUserIfUnassigned`: claims the conversation for `user_id` while neither a user
/// nor a team is assigned (`assigned_at` `None` = now). Empty = did not claim.
pub fn assign_user_if_unassigned(
    _conn: &Connection,
    _workspace_id: i64,
    _conversation_id: i64,
    _user_id: i64,
    _assigned_at: Option<i64>,
) -> Result<Vec<Conversation>> {
    todo!("P3.2b")
}

/// `assignTargetIfUnassigned`: `assignedUserAt` moves only when a user is assigned.
pub fn assign_target_if_unassigned(
    _conn: &Connection,
    _input: &AssignTargetInput,
) -> Result<Vec<Conversation>> {
    todo!("P3.2b")
}

/// `closeHumanReplyWindow`: records a human reply at `replied_at` and closes the DM
/// waiting window that opened at or before it. `None` when the conversation is not in
/// the workspace.
pub fn close_human_reply_window(
    _conn: &Connection,
    _workspace_id: i64,
    _conversation_id: i64,
    _replied_at: i64,
) -> Result<Option<CloseHumanReplyWindowResult>> {
    todo!("P3.2b")
}
