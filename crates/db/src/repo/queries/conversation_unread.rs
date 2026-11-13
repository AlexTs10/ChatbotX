//! Port of `queries/conversation-unread.ts` (`conversationUnreadWhere`): the unread
//! predicate of a `Conversation` row, as SQL over the given table alias. Same semantics as
//! the raw-SQL unread predicate of the contact filter.

/// `conversationUnreadWhere` over `alias` (`""` = unqualified columns): never read and
/// with activity, or activity after the last read.
pub fn conversation_unread_where(alias: &str) -> String {
    let p = if alias.is_empty() {
        String::new()
    } else {
        format!("{alias}.")
    };
    format!(
        "(({p}\"agentLastReadAt\" IS NULL AND {p}\"lastActivityAt\" IS NOT NULL) OR {p}\"lastActivityAt\" > {p}\"agentLastReadAt\")"
    )
}
