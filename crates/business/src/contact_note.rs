//! `packages/business/src/contact-note/service.ts`: internal notes of a
//! contact. Private queries on `"ContactNote"` / `"Contact"`. No read cache.

use chatbotx_db::Db;
use chatbotx_db::repo::models::ContactNote;
use chatbotx_db::repo::{Model, new_id, now_ms};
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, params};

use crate::errors::{BusinessError, Result};
use crate::tx;

fn repo_err(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    BusinessError::Repo(error.into())
}

/// `contactService.findByIdOrFail`.
fn contact_or_fail(conn: &Connection, workspace_id: i64, contact_id: i64) -> Result<()> {
    conn.query_row(
        r#"SELECT 1 FROM "Contact" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
        params![contact_id, workspace_id],
        |_| Ok(()),
    )
    .optional()
    .map_err(repo_err)?
    .ok_or_else(|| BusinessError::not_found("Contact not found"))
}

fn note(conn: &Connection, contact_id: i64, note_id: i64) -> Result<Option<ContactNote>> {
    conn.query_row(
        &format!(
            r#"{} WHERE "id" = ?1 AND "contactId" = ?2"#,
            ContactNote::select_sql()
        ),
        params![note_id, contact_id],
        ContactNote::from_row,
    )
    .optional()
    .map_err(repo_err)
}

/// `create`: 404 "Contact not found" for a contact outside the workspace.
pub async fn create(
    db: &Db,
    workspace_id: i64,
    contact_id: i64,
    text: &str,
    created_by_id: Option<i64>,
) -> Result<ContactNote> {
    let text = text.to_string();
    tx::write(db, move |conn| {
        contact_or_fail(conn, workspace_id, contact_id)?;
        let id = new_id();
        let now = now_ms();
        conn.execute(
            r#"INSERT INTO "ContactNote" ("id", "createdAt", "updatedAt", "text", "contactId", "createdById")
               VALUES (?1, ?2, ?2, ?3, ?4, ?5)"#,
            params![id, now, text, contact_id, created_by_id],
        )
        .map_err(repo_err)?;
        note(conn, contact_id, id)?.ok_or_else(|| BusinessError::not_found("Contact note not found"))
    })
    .await
}

/// `update`: 404 "Contact note not found" for a note of another contact.
pub async fn update(
    db: &Db,
    workspace_id: i64,
    contact_id: i64,
    note_id: i64,
    text: Option<&str>,
) -> Result<ContactNote> {
    let text = text.map(str::to_string);
    tx::write(db, move |conn| {
        contact_or_fail(conn, workspace_id, contact_id)?;
        if note(conn, contact_id, note_id)?.is_none() {
            return Err(BusinessError::not_found("Contact note not found"));
        }
        match text {
            Some(text) => conn.execute(
                r#"UPDATE "ContactNote" SET "text" = ?1, "updatedAt" = ?2 WHERE "id" = ?3 AND "contactId" = ?4"#,
                params![text, now_ms(), note_id, contact_id],
            ),
            None => conn.execute(
                r#"UPDATE "ContactNote" SET "updatedAt" = ?1 WHERE "id" = ?2 AND "contactId" = ?3"#,
                params![now_ms(), note_id, contact_id],
            ),
        }
        .map_err(repo_err)?;
        note(conn, contact_id, note_id)?.ok_or_else(|| BusinessError::not_found("Contact note not found"))
    })
    .await
}

/// `delete`: 404 "Contact note not found" when no row was deleted.
pub async fn delete(db: &Db, workspace_id: i64, contact_id: i64, note_id: i64) -> Result<()> {
    tx::write(db, move |conn| {
        contact_or_fail(conn, workspace_id, contact_id)?;
        let deleted = conn
            .execute(
                r#"DELETE FROM "ContactNote" WHERE "id" = ?1 AND "contactId" = ?2"#,
                params![note_id, contact_id],
            )
            .map_err(repo_err)?;
        if deleted == 0 {
            return Err(BusinessError::not_found("Contact note not found"));
        }
        Ok(())
    })
    .await
}

/// `listByContactId`: newest first.
pub async fn list_by_contact_id(
    db: &Db,
    workspace_id: i64,
    contact_id: i64,
) -> Result<Vec<ContactNote>> {
    tx::read(db, move |conn| {
        contact_or_fail(conn, workspace_id, contact_id)?;
        let mut stmt = conn
            .prepare(&format!(
                r#"{} WHERE "contactId" = ?1 ORDER BY "createdAt" DESC, "id" DESC"#,
                ContactNote::select_sql()
            ))
            .map_err(repo_err)?;
        let rows = stmt
            .query_map(params![contact_id], ContactNote::from_row)
            .map_err(repo_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(repo_err)?;
        Ok(rows)
    })
    .await
}
