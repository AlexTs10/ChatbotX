//! Port of `repositories/ai-workspace-scope/repository.ts`.

use rusqlite::Connection;

use super::Result;

/// `AI_WORKSPACE_SCOPES`: the AI record kinds whose owning workspace can be resolved
/// from a bare id.
pub const AI_WORKSPACE_SCOPES: &[AiWorkspaceScope] = &[
    AiWorkspaceScope::AiEmbedding,
    AiWorkspaceScope::AiFile,
    AiWorkspaceScope::ConversationEmbedding,
    AiWorkspaceScope::ConversationSource,
];

/// `AiWorkspaceScope`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AiWorkspaceScope {
    AiEmbedding,
    AiFile,
    ConversationEmbedding,
    ConversationSource,
}

impl AiWorkspaceScope {
    /// The TS scope name.
    pub fn as_str(self) -> &'static str {
        match self {
            AiWorkspaceScope::AiEmbedding => "aiEmbedding",
            AiWorkspaceScope::AiFile => "aiFile",
            AiWorkspaceScope::ConversationEmbedding => "conversationEmbedding",
            AiWorkspaceScope::ConversationSource => "conversationSource",
        }
    }

    /// The table each scope reads (`workspaceIdReaders`).
    pub fn table(self) -> &'static str {
        match self {
            AiWorkspaceScope::AiEmbedding => "AIEmbedding",
            AiWorkspaceScope::AiFile => "AIFile",
            AiWorkspaceScope::ConversationEmbedding => "AIConversationEmbedding",
            AiWorkspaceScope::ConversationSource => "AIConversationSource",
        }
    }
}

/// `AiWorkspaceScopeRepository.findWorkspaceId`: a primary-key read of only
/// `workspaceId`; `None` when the record no longer exists.
pub fn find_workspace_id(
    conn: &Connection,
    id: i64,
    scope: AiWorkspaceScope,
) -> Result<Option<i64>> {
    let sql = format!(
        "SELECT \"workspaceId\" FROM \"{}\" WHERE \"id\" = ?1 LIMIT 1",
        scope.table()
    );
    let mut stmt = conn.prepare_cached(&sql)?;
    let mut rows = stmt.query([id])?;
    match rows.next()? {
        Some(row) => Ok(Some(row.get(0)?)),
        None => Ok(None),
    }
}
