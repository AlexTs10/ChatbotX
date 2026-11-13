//! Shared fixtures of the P3.2b slices of implementer 7 (ai-handover-bulk-eligibility and
//! later), on top of `common/mod.rs` (locked).
//!
//! Include after `common`:
//! ```ignore
//! #[path = "common/repo12.rs"]
//! mod repo12;
//! ```

#![allow(dead_code)]

use chatbotx_db::Db;
use chatbotx_db::repo::queries::ai_handover_bulk_eligibility::{
    AiHandoverBulkAction, BulkEligibilityInput,
};
use chatbotx_db::repo::sql::Sql;
use chatbotx_db::rusqlite::params;

/// `NOW` of the TS suites (`2026-10-02T12:00:00.000Z`).
pub const NOW: i64 = 1_790_942_400_000;
/// `REQUESTED_AT` (`2026-10-02T11:00:00.000Z`).
pub const REQUESTED_AT: i64 = NOW - HOUR_MS;
/// `SINCE` of the unit suite (`2026-09-25T12:00:00.000Z`).
pub const SINCE: i64 = NOW - 7 * DAY_MS;
pub const HOUR_MS: i64 = 60 * 60 * 1000;
pub const DAY_MS: i64 = 24 * HOUR_MS;

/// `hoursAgo`.
pub fn hours_ago(hours: i64) -> i64 {
    NOW - hours * HOUR_MS
}

/// The TS `input()`: `disable` at `REQUESTED_AT` / `NOW` / `SINCE`.
pub fn input() -> BulkEligibilityInput {
    BulkEligibilityInput {
        action: AiHandoverBulkAction::Disable,
        requested_at: REQUESTED_AT,
        now: NOW,
        last_incoming_since: SINCE,
    }
}

/// [`input`] with `action`.
pub fn input_for(action: AiHandoverBulkAction) -> BulkEligibilityInput {
    BulkEligibilityInput { action, ..input() }
}

/// The thread-control columns of one `ContactInbox` (the TS `Thread`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Thread {
    pub state: Option<&'static str>,
    pub role: Option<&'static str>,
    pub expires_at: Option<i64>,
    pub updated_at: Option<i64>,
    pub last_incoming_at: Option<i64>,
}

/// The TS `thread(id, {})` defaults: no state, active one hour ago.
pub fn thread() -> Thread {
    Thread {
        state: None,
        role: None,
        expires_at: None,
        updated_at: None,
        last_incoming_at: Some(hours_ago(1)),
    }
}

/// The ids of `threads` that `conditions` selects, by id, on a TEMP `"ContactInbox"`
/// table (text ids) that shadows the real one and is dropped after the read (the TS
/// integration suite's rolled-back temp table).
pub async fn shadow_eligible(
    db: &Db,
    threads: Vec<(&'static str, Thread)>,
    conditions: Sql,
) -> Vec<String> {
    db.write(move |tx| {
        tx.execute_batch(
            r#"CREATE TEMP TABLE "ContactInbox" (
                 "id" TEXT, "sourceId" TEXT, "threadControlState" TEXT, "threadOwnerRole" TEXT,
                 "threadOwnerExpiresAt" INTEGER, "threadControlUpdatedAt" INTEGER,
                 "lastIncomingMessageAt" INTEGER)"#,
        )?;
        for (id, t) in &threads {
            tx.execute(
                r#"INSERT INTO temp."ContactInbox" VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"#,
                params![
                    id,
                    format!("psid-{id}"),
                    t.state,
                    t.role,
                    t.expires_at,
                    t.updated_at,
                    t.last_incoming_at
                ],
            )?;
        }
        let sql = format!(
            r#"SELECT "id" FROM "ContactInbox" WHERE {} ORDER BY "id""#,
            conditions.text
        );
        let ids = {
            let mut stmt = tx.prepare(&sql)?;
            stmt.query_map(
                chatbotx_db::rusqlite::params_from_iter(conditions.params.iter()),
                |r| r.get::<_, String>(0),
            )?
            .collect::<Result<Vec<_>, _>>()?
        };
        tx.execute_batch(r#"DROP TABLE temp."ContactInbox""#)?;
        Ok(ids)
    })
    .await
    .expect("shadow eligible")
}

/// Inserts a `Contact` of `workspace_id` and its `ContactInbox` on `inbox_id` (source
/// `psid-<contact_inbox_id>`) with the thread columns of `t`.
pub async fn put_bulk_thread(
    db: &Db,
    contact_inbox_id: i64,
    contact_id: i64,
    workspace_id: i64,
    inbox_id: i64,
    t: Thread,
) {
    db.write(move |tx| {
        tx.execute(
            r#"INSERT OR IGNORE INTO "Contact" ("id", "workspaceId") VALUES (?1, ?2)"#,
            params![contact_id, workspace_id],
        )?;
        tx.execute(
            r#"INSERT INTO "ContactInbox" ("id", "originalContactId", "contactId", "inboxId", "channel",
                 "source", "sourceId", "threadControlState", "threadOwnerRole", "threadOwnerExpiresAt",
                 "threadControlUpdatedAt", "lastIncomingMessageAt")
               VALUES (?1, ?2, ?2, ?3, 'messenger', 'messenger', ?4, ?5, ?6, ?7, ?8, ?9)"#,
            params![
                contact_inbox_id,
                contact_id,
                inbox_id,
                format!("psid-{contact_inbox_id}"),
                t.state,
                t.role,
                t.expires_at,
                t.updated_at,
                t.last_incoming_at
            ],
        )?;
        Ok(())
    })
    .await
    .expect("put bulk thread");
}

/// Inserts a `Conversation` of `contact_id` in `workspace_id` (`source_id = None` is the
/// direct-message one) with the assignment columns.
pub async fn put_conversation(
    db: &Db,
    id: i64,
    workspace_id: i64,
    contact_id: i64,
    source_id: Option<&'static str>,
    assigned_inbox_team_id: Option<i64>,
    assigned_user_id: Option<i64>,
) {
    db.write(move |tx| {
        tx.execute(
            r#"INSERT INTO "Conversation" ("id", "workspaceId", "contactId", "sourceId",
                 "assignedInboxTeamId", "assignedUserId") VALUES (?1, ?2, ?3, ?4, ?5, ?6)"#,
            params![
                id,
                workspace_id,
                contact_id,
                source_id,
                assigned_inbox_team_id,
                assigned_user_id
            ],
        )?;
        Ok(())
    })
    .await
    .expect("put conversation");
}

/// Inserts an `InboxTeam` of `workspace_id`.
pub async fn put_inbox_team(db: &Db, id: i64, workspace_id: i64) {
    db.write(move |tx| {
        tx.execute(
            r#"INSERT INTO "InboxTeam" ("id", "workspaceId", "name") VALUES (?1, ?2, 'team')"#,
            params![id, workspace_id],
        )?;
        Ok(())
    })
    .await
    .expect("put inbox team");
}
