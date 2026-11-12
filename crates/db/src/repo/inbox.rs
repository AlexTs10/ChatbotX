//! Port of `repositories/inbox/repository.ts` (`touchThreadControlSeen`; the rest is
//! listed as not ported yet in `conversion/specs/P3.2.md`).

use rusqlite::Connection;

use super::Result;
use super::partials::thread_control::THREAD_CONTROL_SEEN_REFRESH_MS;

/// `touchThreadControlSeen`: writes `threadControlSeenAt` only when missing or older than
/// the refresh interval (workspace-scoped). `true` when written.
pub fn touch_thread_control_seen(
    conn: &Connection,
    workspace_id: i64,
    inbox_id: i64,
    seen_at: i64,
) -> Result<bool> {
    let n = conn.execute(
        "UPDATE \"Inbox\" SET \"threadControlSeenAt\" = ?3 WHERE \"id\" = ?2 AND \"workspaceId\" = ?1 \
         AND (\"threadControlSeenAt\" IS NULL OR \"threadControlSeenAt\" < ?4)",
        rusqlite::params![workspace_id, inbox_id, seen_at, seen_at - THREAD_CONTROL_SEEN_REFRESH_MS],
    )?;
    Ok(n > 0)
}
