//! Shared fixtures of the P3.3 / P3.2b repository ports (on top of `common/repo.rs`).
//!
//! Include after `repo_common`:
//! ```ignore
//! #[path = "common/repo3.rs"]
//! mod repo3;
//! ```

#![allow(dead_code)]

use chatbotx_db::DbError;
use chatbotx_db::rusqlite::{Connection, params};

/// Inserts one Inbox (`channel`, sourceId `<channel>-<id>`) and one `IntegrationMessenger`
/// on it (`personas` = JSON text). Returns the integration id.
pub fn integration_messenger(
    tx: &Connection,
    id: i64,
    workspace_id: i64,
    inbox_id: i64,
    personas: &str,
) -> Result<i64, DbError> {
    inbox(tx, inbox_id, workspace_id, "messenger")?;
    tx.execute(
        r#"INSERT INTO "IntegrationMessenger" ("id", "auth", "pageId", "name", "conversationStarters",
             "persistentMenus", "personas", "workspaceId", "inboxId")
           VALUES (?1, '{}', ?2, 'Page', '[]', '[]', ?3, ?4, ?5)"#,
        params![id, format!("page-{id}"), personas, workspace_id, inbox_id],
    )?;
    Ok(id)
}

/// Inserts one Inbox and one `IntegrationWhatsapp` (`wabaId` = `waba_id`).
pub fn integration_whatsapp(
    tx: &Connection,
    id: i64,
    workspace_id: i64,
    inbox_id: i64,
    waba_id: &str,
) -> Result<i64, DbError> {
    inbox(tx, inbox_id, workspace_id, "whatsapp")?;
    tx.execute(
        r#"INSERT INTO "IntegrationWhatsapp" ("id", "auth", "phoneNumberId", "wabaId", "businessId", "name",
             "workspaceId", "inboxId")
           VALUES (?1, '{}', ?2, ?3, 'business-1', 'WhatsApp', ?4, ?5)"#,
        params![id, format!("phone-{id}"), waba_id, workspace_id, inbox_id],
    )?;
    Ok(id)
}

/// Inserts one Inbox and one `IntegrationInstagram`.
pub fn integration_instagram(
    tx: &Connection,
    id: i64,
    workspace_id: i64,
    inbox_id: i64,
) -> Result<i64, DbError> {
    inbox(tx, inbox_id, workspace_id, "instagram")?;
    tx.execute(
        r#"INSERT INTO "IntegrationInstagram" ("id", "auth", "igId", "pageId", "name", "username",
             "conversationStarters", "persistentMenus", "workspaceId", "inboxId")
           VALUES (?1, '{}', ?2, ?3, 'IG', 'ig', '[]', '[]', ?4, ?5)"#,
        params![
            id,
            format!("ig-{id}"),
            format!("page-{id}"),
            workspace_id,
            inbox_id
        ],
    )?;
    Ok(id)
}

/// Inserts one Inbox of `workspace_id` on `channel` (sourceId `<channel>-<id>`).
pub fn inbox(tx: &Connection, id: i64, workspace_id: i64, channel: &str) -> Result<usize, DbError> {
    Ok(tx.execute(
        r#"INSERT INTO "Inbox" ("id", "name", "channel", "sourceId", "workspaceId") VALUES (?1, ?2, ?3, ?4, ?5)"#,
        params![id, format!("Inbox {id}"), channel, format!("{channel}-{id}"), workspace_id],
    )?)
}

/// Inserts one Flow of `workspace_id`.
pub fn flow(tx: &Connection, id: i64, workspace_id: i64) -> Result<usize, DbError> {
    Ok(tx.execute(
        r#"INSERT INTO "Flow" ("id", "name", "workspaceId") VALUES (?1, ?2, ?3)"#,
        params![id, format!("Flow {id}"), workspace_id],
    )?)
}
